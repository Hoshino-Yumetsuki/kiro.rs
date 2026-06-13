//! Test harness for Anthropic API integration tests.
//!
//! Provides utilities to build the same axum [`Router`] used in production
//! (but without a real `KiroProvider`, so no live API calls are made).
//!
//! # Example
//!
//! ```rust,ignore
//! use common::{build_test_app, request, body_json};
//! use axum::body::Body;
//! use http::Request;
//!
//! #[tokio::test]
//! async fn my_test() {
//!     let app = build_test_app();
//!     let req = Request::builder()
//!         .uri("/v1/models")
//!         .header("x-api-key", common::TEST_API_KEY)
//!         .body(Body::empty())
//!         .unwrap();
//!     let resp = request(app, req).await;
//!     assert_eq!(resp.status(), 200);
//! }
//! ```

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use bytes::Bytes;
use http::Request;
use http::Response;
use parking_lot::RwLock;

use kiro_rs::anthropic;
use kiro_rs::kiro;
use kiro_rs::model::config::{CompressionConfig, PromptCacheMode};

/// Fixed API key used in tests.
pub const TEST_API_KEY: &str = "test-api-key";

/// Build a test [`Router`] with the same construction as `main.rs`,
/// but without a real `KiroProvider` (no live upstream calls).
pub fn build_test_app() -> Router {
    let compression_config = Arc::new(RwLock::new(CompressionConfig::default()));
    let prompt_cache_runtime = Arc::new(RwLock::new(anthropic::PromptCacheRuntime::new(
        300,
        PromptCacheMode::Simulated,
    )));
    let rewriter_config = Arc::new(RwLock::new(anthropic::rewriter::RewriterConfig::default()));

    anthropic::create_router_with_provider(
        TEST_API_KEY,
        None::<Arc<kiro::provider::KiroProvider>>,
        None::<String>,
        compression_config,
        prompt_cache_runtime,
        rewriter_config,
    )
}

/// Execute a request against the app and return the response.
///
/// Each call consumes the `Router` — create a new one per test with
/// [`build_test_app`].
pub async fn request(app: Router, req: Request<Body>) -> Response<Body> {
    use tower::util::ServiceExt;
    app.oneshot(req).await.unwrap()
}

/// Extract the response body as [`Bytes`].
#[allow(dead_code)]
pub async fn body_bytes(resp: Response<Body>) -> Bytes {
    use futures::StreamExt;

    let mut buf = Vec::new();
    let mut stream = resp.into_body().into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();
        buf.extend_from_slice(&chunk);
    }
    Bytes::from(buf)
}

/// Extract the response body as [`serde_json::Value`].
#[allow(dead_code)]
pub async fn body_json(resp: Response<Body>) -> serde_json::Value {
    let bytes = body_bytes(resp).await;
    serde_json::from_slice(&bytes).unwrap()
}
