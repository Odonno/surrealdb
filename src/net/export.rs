use std::ops::Deref;

use super::AppState;
use super::error::ResponseError;
use super::headers::ContentType;
use crate::net::error::Error as NetError;
use anyhow::Result;
use axum::Router;
use axum::body::Body;
use axum::response::IntoResponse;
use axum::routing::options;
use axum::{Extension, response::Response};
use axum_extra::TypedHeader;
use bytes::Bytes;
use http::StatusCode;
use surrealdb::dbs::Session;
use surrealdb::dbs::capabilities::RouteTarget;
use surrealdb::iam::Action::View;
use surrealdb::iam::ResourceKind::Any;
use surrealdb::iam::check::check_ns_db;
use surrealdb::kvs::export;
use surrealdb::rpc::format::Format;

pub(super) fn router<S>() -> Router<S>
where
	S: Clone + Send + Sync + 'static,
{
	Router::new().route("/export", options(|| async {}).get(get_handler).post(post_handler))
}

async fn get_handler(
	Extension(state): Extension<AppState>,
	Extension(session): Extension<Session>,
) -> Result<impl IntoResponse, ResponseError> {
	let cfg = export::Config::default();
	handle_inner(state, session, cfg).await
}

async fn post_handler(
	Extension(state): Extension<AppState>,
	Extension(session): Extension<Session>,
	content_type: TypedHeader<ContentType>,
	body: Bytes,
) -> Result<impl IntoResponse, ResponseError> {
	let fmt = content_type.deref();
	let fmt: Format = fmt.into();
	let val = fmt.parse_value(body)?;
	let cfg = export::Config::from_value(&val).map_err(ResponseError)?;
	handle_inner(state, session, cfg).await
}

async fn handle_inner(
	state: AppState,
	session: Session,
	cfg: export::Config,
) -> Result<impl IntoResponse, ResponseError> {
	// Get the datastore reference
	let db = &state.datastore;
	// Check if capabilities allow querying the requested HTTP route
	if !db.allows_http_route(&RouteTarget::Export) {
		warn!("Capabilities denied HTTP route request attempt, target: '{}'", &RouteTarget::Export);
		return Err(NetError::ForbiddenRoute(RouteTarget::Export.to_string()).into());
	}
	// Create a chunked response
	let (chn, body_stream) = surrealdb::channel::bounded::<Result<Bytes>>(1);
	let body = Body::from_stream(body_stream);
	// Ensure a NS and DB are set
	let (nsv, dbv) = check_ns_db(&session).map_err(ResponseError)?;
	// Check the permissions level
	db.check(&session, View, Any.on_db(&nsv, &dbv)).map_err(ResponseError)?;
	// Create a new bounded channel
	let (snd, rcv) = surrealdb::channel::bounded(1);
	// Start the export task
	let task = db.export_with_config(&session, snd, cfg).await.map_err(ResponseError)?;
	// Spawn a new database export job
	tokio::spawn(task);
	// Process all chunk values
	tokio::spawn(async move {
		while let Ok(v) = rcv.recv().await {
			let _ = chn.send(Ok(Bytes::from(v))).await;
		}
	});
	// Return the chunked body
	Ok(Response::builder().status(StatusCode::OK).body(body).unwrap())
}
