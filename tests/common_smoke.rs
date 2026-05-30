mod common;

use axum::body::Body;
use http::Request;

#[tokio::test]
async fn rejects_unauthenticated_post_messages() {
    let app = common::build_test_app();

    let body = serde_json::json!({
        "model": "claude-sonnet-4-20250514",
        "max_tokens": 10,
        "messages": [{"role": "user", "content": "hi"}],
    });

    let req = Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();

    let resp = common::request(app, req).await;
    assert_eq!(resp.status(), 401);

    let json = common::body_json(resp).await;
    assert_eq!(json["error"]["type"], "authentication_error");
    assert_eq!(json["error"]["message"], "Invalid API key");
}
