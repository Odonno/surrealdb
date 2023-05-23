use crate::ctx::Context;
use crate::dbs::Options;
use crate::dbs::Statement;
use crate::dbs::Transaction;
use crate::doc::Document;
use crate::err::Error;
use crate::idx::planner::executor::QueryExecutor;
use crate::sql::value::Value;

impl<'a> Document<'a> {
	pub async fn select(
		&self,
		ctx: &Context<'_>,
		opt: &Options,
		txn: &Transaction,
		stm: &Statement<'_>,
		exe: Option<&QueryExecutor>,
	) -> Result<Value, Error> {
		// Check if record exists
		self.empty(ctx, opt, txn, stm).await?;
		// Check where clause
		self.check(ctx, opt, txn, stm, exe).await?;
		// Check if allowed
		self.allow(ctx, opt, txn, stm).await?;
		// Yield document
		self.pluck(ctx, opt, txn, stm).await
	}
}
