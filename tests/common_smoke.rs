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

#[tokio::test]
async fn get_models_returns_anthropic_shape() {
    let app = common::build_test_app();

    let req = Request::builder()
        .uri("/v1/models")
        .header("x-api-key", common::TEST_API_KEY)
        .body(Body::empty())
        .unwrap();

    let resp = common::request(app, req).await;
    assert_eq!(resp.status(), 200);

    let json = common::body_json(resp).await;

    // Top-level shape
    assert!(json["data"].is_array(), "data should be an array");
    assert_eq!(json["has_more"], false, "has_more should be false");
    assert!(json["first_id"].is_string(), "first_id should be present");
    assert!(json["last_id"].is_string(), "last_id should be present");
    assert!(
        json.get("object").is_none(),
        "should not have object field"
    );

    // Each model info entry must match Anthropic ModelInfo shape
    let data = json["data"].as_array().unwrap();
    assert!(!data.is_empty(), "should have at least one model");

    for (i, model) in data.iter().enumerate() {
        let fields = model.as_object().expect("each model should be an object");
        assert!(
            fields.contains_key("id"),
            "model[{}] missing id",
            i
        );
        assert_eq!(
            model["type"], "model",
            "model[{}] type should be 'model'",
            i
        );
        assert!(
            fields.contains_key("display_name"),
            "model[{}] missing display_name",
            i
        );
        assert!(
            model["created_at"].is_i64() || model["created_at"].is_u64(),
            "model[{}] created_at should be an integer",
            i
        );
        assert!(
            fields.get("object").is_none(),
            "model[{}] should not have object field",
            i
        );
        assert!(
            fields.get("owned_by").is_none(),
            "model[{}] should not have owned_by field",
            i
        );
    }

    // first_id should match first model's id, last_id should match last model's id
    assert_eq!(json["first_id"], data[0]["id"], "first_id mismatch");
    assert_eq!(
        json["last_id"],
        data[data.len() - 1]["id"],
        "last_id mismatch"
    );
}

#[tokio::test]
async fn get_model_by_id_returns_200_and_model() {
    let app = common::build_test_app();

    let req = Request::builder()
        .uri("/v1/models/claude-sonnet-4-6")
        .header("x-api-key", common::TEST_API_KEY)
        .body(Body::empty())
        .unwrap();

    let resp = common::request(app, req).await;
    assert_eq!(resp.status(), 200);

    // Extract header before consuming body
    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("response should have x-request-id header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        request_id.starts_with("req_"),
        "x-request-id should start with req_"
    );

    let json = common::body_json(resp).await;
    assert_eq!(json["id"], "claude-sonnet-4-6");
    assert_eq!(json["type"], "model");
    assert_eq!(json["display_name"], "Claude Sonnet 4.6");
    assert!(json["created_at"].is_i64() || json["created_at"].is_u64());
}

#[tokio::test]
async fn get_model_by_id_returns_404_on_miss() {
    let app = common::build_test_app();

    let req = Request::builder()
        .uri("/v1/models/nonexistent-model")
        .header("x-api-key", common::TEST_API_KEY)
        .body(Body::empty())
        .unwrap();

    let resp = common::request(app, req).await;
    assert_eq!(resp.status(), 404);

    // Extract header before consuming body
    let header_value = resp
        .headers()
        .get("x-request-id")
        .expect("404 response should have x-request-id header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        header_value.starts_with("req_"),
        "header x-request-id should start with req_"
    );

    let json = common::body_json(resp).await;

    // Error envelope shape
    assert_eq!(json["type"], "error");
    assert_eq!(json["error"]["type"], "not_found_error");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent-model"),
        "error message should contain the requested model ID"
    );

    // request_id in body matches header
    assert_eq!(
        json["request_id"].as_str().unwrap(),
        &header_value,
        "body request_id should match header"
    );
}
