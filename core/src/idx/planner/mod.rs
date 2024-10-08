pub mod checker;
pub(crate) mod executor;
pub(crate) mod iterators;
pub(in crate::idx) mod knn;
pub(crate) mod plan;
pub(in crate::idx) mod rewriter;
pub(in crate::idx) mod tree;

use crate::ctx::Context;
use crate::dbs::{Iterable, Iterator, Options};
use crate::err::Error;
use crate::idx::planner::executor::{InnerQueryExecutor, IteratorEntry, QueryExecutor};
use crate::idx::planner::iterators::IteratorRef;
use crate::idx::planner::knn::KnnBruteForceResults;
use crate::idx::planner::plan::{Plan, PlanBuilder};
use crate::idx::planner::tree::Tree;
use crate::sql::with::With;
use crate::sql::{Cond, Orders, Table};
use reblessive::tree::Stk;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

pub(crate) struct QueryPlanner {
	opt: Arc<Options>,
	with: Option<Arc<With>>,
	cond: Option<Arc<Cond>>,
	order: Option<Arc<Orders>>,
	/// There is one executor per table
	executors: HashMap<String, QueryExecutor>,
	requires_distinct: bool,
	fallbacks: Vec<String>,
	iteration_workflow: Vec<IterationStage>,
	iteration_index: AtomicU8,
	orders: Vec<IteratorRef>,
}

impl QueryPlanner {
	pub(crate) fn new(
		opt: Arc<Options>,
		with: Option<Arc<With>>,
		cond: Option<Arc<Cond>>,
		order: Option<Arc<Orders>>,
	) -> Self {
		Self {
			opt,
			with,
			cond,
			order,
			executors: HashMap::default(),
			requires_distinct: false,
			fallbacks: vec![],
			iteration_workflow: Vec::default(),
			iteration_index: AtomicU8::new(0),
			orders: vec![],
		}
	}

	pub(crate) async fn add_iterables(
		&mut self,
		stk: &mut Stk,
		ctx: &Context,
		t: Table,
		it: &mut Iterator,
	) -> Result<(), Error> {
		let mut is_table_iterator = false;
		let mut tree = Tree::build(
			stk,
			ctx,
			&self.opt,
			&t,
			self.cond.as_ref().map(|w| w.as_ref()),
			self.with.as_ref().map(|c| c.as_ref()),
			self.order.as_ref().map(|o| o.as_ref()),
		)
		.await?;

		let is_knn = !tree.knn_expressions.is_empty();
		let order = tree.index_map.order_limit.take();
		let mut exe = InnerQueryExecutor::new(
			stk,
			ctx,
			&self.opt,
			&t,
			tree.index_map,
			tree.knn_expressions,
			tree.knn_brute_force_expressions,
			tree.knn_condition,
		)
		.await?;
		match PlanBuilder::build(
			tree.root,
			self.with.as_ref().map(|w| w.as_ref()),
			tree.with_indexes,
			order,
		)? {
			Plan::SingleIndex(exp, io) => {
				if io.require_distinct() {
					self.requires_distinct = true;
				}
				let is_order = exp.is_none();
				let ir = exe.add_iterator(IteratorEntry::Single(exp, io));
				self.add(t.clone(), Some(ir), exe, it);
				if is_order {
					self.orders.push(ir);
				}
			}
			Plan::MultiIndex(non_range_indexes, ranges_indexes) => {
				for (exp, io) in non_range_indexes {
					let ie = IteratorEntry::Single(Some(exp), io);
					let ir = exe.add_iterator(ie);
					it.ingest(Iterable::Index(t.clone(), ir));
				}
				for (ixr, rq) in ranges_indexes {
					let ie = IteratorEntry::Range(rq.exps, ixr, rq.from, rq.to);
					let ir = exe.add_iterator(ie);
					it.ingest(Iterable::Index(t.clone(), ir));
				}
				self.requires_distinct = true;
				self.add(t.clone(), None, exe, it);
			}
			Plan::SingleIndexRange(ixn, rq) => {
				let ir = exe.add_iterator(IteratorEntry::Range(rq.exps, ixn, rq.from, rq.to));
				self.add(t.clone(), Some(ir), exe, it);
			}
			Plan::TableIterator(fallback) => {
				if let Some(fallback) = fallback {
					self.fallbacks.push(fallback);
				}
				self.add(t.clone(), None, exe, it);
				it.ingest(Iterable::Table(t));
				is_table_iterator = true;
			}
		}
		if is_knn && is_table_iterator {
			self.iteration_workflow = vec![IterationStage::CollectKnn, IterationStage::BuildKnn];
		} else {
			self.iteration_workflow = vec![IterationStage::Iterate(None)];
		}
		Ok(())
	}

	fn add(
		&mut self,
		tb: Table,
		irf: Option<IteratorRef>,
		exe: InnerQueryExecutor,
		it: &mut Iterator,
	) {
		self.executors.insert(tb.0.clone(), exe.into());
		if let Some(irf) = irf {
			it.ingest(Iterable::Index(tb, irf));
		}
	}
	pub(crate) fn has_executors(&self) -> bool {
		!self.executors.is_empty()
	}

	pub(crate) fn get_query_executor(&self, tb: &str) -> Option<&QueryExecutor> {
		self.executors.get(tb)
	}

	pub(crate) fn requires_distinct(&self) -> bool {
		self.requires_distinct
	}

	pub(crate) fn fallbacks(&self) -> &Vec<String> {
		&self.fallbacks
	}

	pub(crate) fn is_order(&self, irf: &IteratorRef) -> bool {
		self.orders.contains(irf)
	}

	pub(crate) async fn next_iteration_stage(&self) -> Option<IterationStage> {
		let pos = self.iteration_index.fetch_add(1, Ordering::Relaxed);
		match self.iteration_workflow.get(pos as usize) {
			Some(IterationStage::BuildKnn) => {
				Some(IterationStage::Iterate(Some(self.build_bruteforce_knn_results().await)))
			}
			is => is.cloned(),
		}
	}

	async fn build_bruteforce_knn_results(&self) -> KnnBruteForceResults {
		let mut results = HashMap::with_capacity(self.executors.len());
		for (tb, exe) in &self.executors {
			results.insert(tb.clone(), exe.build_bruteforce_knn_result().await);
		}
		results.into()
	}
}

#[derive(Clone)]
pub(crate) enum IterationStage {
	Iterate(Option<KnnBruteForceResults>),
	CollectKnn,
	BuildKnn,
}
