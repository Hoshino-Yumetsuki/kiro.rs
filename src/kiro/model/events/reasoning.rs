//! 推理内容事件
//!
//! 处理 reasoningContentEvent 类型的事件（Thinking 模式的原生推理内容）

use serde::{Deserialize, Serialize};

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::EventPayload;

/// 推理内容事件
///
/// 包含 AI 模型的推理/思考内容，由 Kiro API 在启用 thinking 时返回。
///
/// # 字段说明
///
/// - `text`: 推理文本内容片段
/// - `signature`: 推理内容签名（用于 Anthropic 格式的 signature_delta）
/// - `redacted_content`: 加密的推理内容（用于 Anthropic 格式的 redacted_thinking）
///
/// # 示例
///
/// ```rust
/// use kiro_rs::kiro::model::events::ReasoningContentEvent;
///
/// let json = r#"{"text":"Let me think about this..."}"#;
/// let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
/// assert_eq!(event.text.as_deref(), Some("Let me think about this..."));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningContentEvent {
    /// 推理文本内容片段
    #[serde(default)]
    pub text: Option<String>,

    /// 推理内容签名
    #[serde(default)]
    pub signature: Option<String>,

    /// 加密的推理内容（redacted thinking）
    #[serde(default)]
    pub redacted_content: Option<String>,
}

impl EventPayload for ReasoningContentEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_text_only() {
        let json = r#"{"text":"Let me think about this..."}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.text.as_deref(), Some("Let me think about this..."));
        assert!(event.signature.is_none());
        assert!(event.redacted_content.is_none());
    }

    #[test]
    fn test_deserialize_with_signature() {
        let json = r#"{"text":"","signature":"sig_abc123"}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.text.as_deref(), Some(""));
        assert_eq!(event.signature.as_deref(), Some("sig_abc123"));
    }

    #[test]
    fn test_deserialize_redacted() {
        let json = r#"{"redactedContent":"encrypted_data_here"}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert!(event.text.is_none());
        assert_eq!(
            event.redacted_content.as_deref(),
            Some("encrypted_data_here")
        );
    }

    #[test]
    fn test_deserialize_all_fields() {
        let json =
            r#"{"text":"thinking...","signature":"sig_xyz","redactedContent":"redacted_data"}"#;
        let event: ReasoningContentEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.text.as_deref(), Some("thinking..."));
        assert_eq!(event.signature.as_deref(), Some("sig_xyz"));
        assert_eq!(event.redacted_content.as_deref(), Some("redacted_data"));
    }
}
