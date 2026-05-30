//! Integration tests for the `x-anthropic-compat-warning` response header
//! emitted when a request sets `tool_choice` to a non-default value.
//!
//! The proxy currently drops `tool_choice` before forwarding to Kiro, so it
//! signals this loss of fidelity via a response header rather than rejecting
//! the request.

mod common;

use axum::body::Body;
use http::Request;

const WARNING_HEADER: &str = "x-anthropic-compat-warning";
const WARNING_VALUE: &str = "tool_choice ignored by upstream";

fn build_request(body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .header("x-api-key", common::TEST_API_KEY)
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn base_payload() -> serde_json::Value {
    serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "hi"}],
    })
}

#[tokio::test]
async fn no_tool_choice_omits_warning_header() {
    let app = common::build_test_app();
    let resp = common::request(app, build_request(base_payload())).await;
    assert!(
        resp.headers().get(WARNING_HEADER).is_none(),
        "header should be absent when tool_choice is missing"
    );
}

#[tokio::test]
async fn tool_choice_auto_omits_warning_header() {
    let app = common::build_test_app();
    let mut body = base_payload();
    body["tool_choice"] = serde_json::json!({"type": "auto"});
    let resp = common::request(app, build_request(body)).await;
    assert!(
        resp.headers().get(WARNING_HEADER).is_none(),
        "header should be absent when tool_choice is the default {{type:auto}}"
    );
}

#[tokio::test]
async fn tool_choice_any_emits_warning_header() {
    let app = common::build_test_app();
    let mut body = base_payload();
    body["tool_choice"] = serde_json::json!({"type": "any"});
    let resp = common::request(app, build_request(body)).await;
    let header = resp
        .headers()
        .get(WARNING_HEADER)
        .expect("header should be present for tool_choice {type:any}");
    assert_eq!(header.to_str().unwrap(), WARNING_VALUE);
}

#[tokio::test]
async fn tool_choice_none_emits_warning_header() {
    let app = common::build_test_app();
    let mut body = base_payload();
    body["tool_choice"] = serde_json::json!({"type": "none"});
    let resp = common::request(app, build_request(body)).await;
    let header = resp
        .headers()
        .get(WARNING_HEADER)
        .expect("header should be present for tool_choice {type:none}");
    assert_eq!(header.to_str().unwrap(), WARNING_VALUE);
}

#[tokio::test]
async fn tool_choice_specific_tool_emits_warning_header() {
    let app = common::build_test_app();
    let mut body = base_payload();
    body["tool_choice"] = serde_json::json!({"type": "tool", "name": "get_weather"});
    let resp = common::request(app, build_request(body)).await;
    let header = resp
        .headers()
        .get(WARNING_HEADER)
        .expect("header should be present for tool_choice {type:tool}");
    assert_eq!(header.to_str().unwrap(), WARNING_VALUE);
}

#[tokio::test]
async fn streaming_request_with_non_default_tool_choice_emits_warning_header() {
    let app = common::build_test_app();
    let mut body = base_payload();
    body["stream"] = serde_json::json!(true);
    body["tool_choice"] = serde_json::json!({"type": "any"});
    let resp = common::request(app, build_request(body)).await;
    let header = resp
        .headers()
        .get(WARNING_HEADER)
        .expect("header should be present on streaming responses too");
    assert_eq!(header.to_str().unwrap(), WARNING_VALUE);
}

#[tokio::test]
async fn auth_failure_does_not_emit_warning_header() {
    let app = common::build_test_app();
    let mut body = base_payload();
    body["tool_choice"] = serde_json::json!({"type": "any"});
    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = common::request(app, req).await;
    assert_eq!(resp.status(), 401);
    assert!(
        resp.headers().get(WARNING_HEADER).is_none(),
        "header must not be set on auth failures"
    );
}

#[tokio::test]
async fn models_endpoint_does_not_emit_warning_header() {
    let app = common::build_test_app();
    let req = Request::builder()
        .uri("/v1/models")
        .header("x-api-key", common::TEST_API_KEY)
        .body(Body::empty())
        .unwrap();
    let resp = common::request(app, req).await;
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers().get(WARNING_HEADER).is_none(),
        "header must not be set on /v1/models"
    );
}
