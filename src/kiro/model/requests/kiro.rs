//! Kiro 请求类型定义
//!
//! 定义 Kiro API 的主请求结构

use serde::{Deserialize, Serialize};

use super::conversation::ConversationState;

/// Kiro API 请求
///
/// 用于构建发送给 Kiro API 的请求
///
/// # 示例
///
/// ```rust
/// use kiro_rs::kiro::model::requests::kiro::KiroRequest;
/// use kiro_rs::kiro::model::requests::conversation::ConversationState;
///
/// let state: ConversationState = serde_json::from_str(r#"{
///     "conversationId": "conv-123",
///     "agentTaskType": "vibe",
///     "chatTriggerType": "MANUAL",
///     "currentMessage": {"userInputMessage": {
///         "content": "Hello",
///         "modelId": "claude-sonnet-4.5",
///         "images": [],
///         "userInputMessageContext": {}
///     }},
///     "history": []
/// }"#).unwrap();
/// let request = KiroRequest {
///     conversation_state: state,
///     profile_arn: None,
///     additional_model_request_fields: None,
/// };
/// let json = serde_json::to_string(&request).unwrap();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KiroRequest {
    /// 对话状态
    pub conversation_state: ConversationState,
    /// Profile ARN（可选）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_arn: Option<String>,
    /// 额外模型请求字段（thinking 等模型级参数）
    /// 例如: { "thinking": { "type": "adaptive" }, "output_config": { "effort": "high" } }
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_model_request_fields: Option<serde_json::Value>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_kiro_request_deserialize() {
        let json = r#"{
            "conversationState": {
                "conversationId": "conv-456",
                "currentMessage": {
                    "userInputMessage": {
                        "content": "Test message",
                        "modelId": "claude-3-5-sonnet",
                        "userInputMessageContext": {}
                    }
                }
            }
        }"#;

        let request: KiroRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.conversation_state.conversation_id, "conv-456");
        assert_eq!(
            request
                .conversation_state
                .current_message
                .user_input_message
                .content,
            "Test message"
        );
    }
}
