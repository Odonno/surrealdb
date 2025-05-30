use crate::api::Connection;
use crate::api::Result;
use crate::api::Surreal;
use crate::api::method::BoxFuture;
use crate::api::method::Cancel;
use crate::api::method::Commit;
use std::future::IntoFuture;
use std::ops::Deref;
use surrealdb_core::expr::statements::BeginStatement;

/// A beginning of a transaction
#[derive(Debug)]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct Begin<C: Connection> {
	pub(super) client: Surreal<C>,
}

impl<C> IntoFuture for Begin<C>
where
	C: Connection,
{
	type Output = Result<Transaction<C>>;
	type IntoFuture = BoxFuture<'static, Self::Output>;

	fn into_future(self) -> Self::IntoFuture {
		Box::pin(async move {
			self.client.query(BeginStatement::default()).await?;
			Ok(Transaction {
				client: self.client,
			})
		})
	}
}

/// An ongoing transaction
#[derive(Debug)]
#[must_use = "transactions must be committed or cancelled to complete them"]
pub struct Transaction<C: Connection> {
	client: Surreal<C>,
}

impl<C> Transaction<C>
where
	C: Connection,
{
	/// Creates a commit future
	pub fn commit(self) -> Commit<C> {
		Commit {
			client: self.client,
		}
	}

	/// Creates a cancel future
	pub fn cancel(self) -> Cancel<C> {
		Cancel {
			client: self.client,
		}
	}
}

impl<C> Deref for Transaction<C>
where
	C: Connection,
{
	type Target = Surreal<C>;

	fn deref(&self) -> &Self::Target {
		&self.client
	}
}
