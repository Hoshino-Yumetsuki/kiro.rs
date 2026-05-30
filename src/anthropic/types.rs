//! Anthropic API 类型定义

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// === 缓存控制 ===

/// 缓存控制配置
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CacheControl {
    #[serde(rename = "type")]
    pub cache_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

// === 错误响应 ===

/// API 错误响应（Anthropic 错误信封格式）
///
/// 序列化为:
/// ```json
/// {
///     "type": "error",
///     "error": { "type": "invalid_request_error", "message": "..." },
///     "request_id": "req_..."
/// }
/// ```
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    #[serde(rename = "type")]
    pub response_type: String,
    pub error: ErrorDetail,
    pub request_id: String,
}

/// 错误详情
#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

impl ErrorResponse {
    /// 创建新的错误响应（指定 request_id）
    pub fn new(error_type: &str, message: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self {
            response_type: "error".to_string(),
            error: ErrorDetail {
                error_type: error_type.to_string(),
                message: message.into(),
            },
            request_id: request_id.into(),
        }
    }

    /// 创建错误响应（自动生成 request_id）
    ///
    /// 生成格式为 `req_{uuid_v4_simple}` 的 request_id。
    /// T7 将改为传入真实 request_id，此方法届时可移除。
    pub fn without_request_id(error_type: &str, message: impl Into<String>) -> Self {
        let request_id = format!("req_{}", Uuid::new_v4().simple());
        Self::new(error_type, message, request_id)
    }

    /// 创建认证错误响应
    pub fn authentication_error() -> Self {
        Self::without_request_id("authentication_error", "Invalid API key")
    }
}

// === Models 端点类型 ===

/// 模型信息
#[derive(Debug, Serialize)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub owned_by: String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub model_type: String,
    pub max_tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<bool>,
}

/// 模型列表响应
#[derive(Debug, Serialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<Model>,
}

// === Messages 端点类型 ===

/// 最大思考预算 tokens
const MAX_BUDGET_TOKENS: i32 = 128_000;

/// Thinking 配置
///
/// 支持两种模式：
/// - `adaptive`: 自适应思考模式，通过 output_config.effort 控制思考级别
/// - `enabled`: 旧版兼容模式（内部映射为 adaptive + 对应 effort）
/// - `disabled`: 禁用思考
#[derive(Debug, Deserialize, Clone)]
pub struct Thinking {
    #[serde(rename = "type")]
    pub thinking_type: String,
    /// 旧版 budget_tokens 字段，保留用于反序列化兼容性（客户端可能仍发送此字段）
    /// 实际不再使用，thinking 级别通过 output_config.effort 控制
    #[serde(
        default = "default_budget_tokens",
        deserialize_with = "deserialize_budget_tokens"
    )]
    #[allow(dead_code)]
    pub budget_tokens: i32,
}

impl Thinking {
    /// 是否启用了 thinking（enabled 或 adaptive）
    pub fn is_enabled(&self) -> bool {
        self.thinking_type == "enabled" || self.thinking_type == "adaptive"
    }

    /// 判断模型是否支持 additionalModelRequestFields.thinking 参数
    /// 只有 Claude Sonnet 4.5+ 和 Opus 4.5+ 系列模型支持
    /// Haiku 系列不支持 additionalModelRequestFields
    pub fn model_supports_thinking(model: &str) -> bool {
        let lower = model.to_lowercase();
        if !lower.contains("claude") {
            return false;
        }
        // claude-3.x 不支持
        if lower.contains("claude-3-") || lower.contains("claude-3.") {
            return false;
        }
        // haiku 系列不支持 additionalModelRequestFields
        if lower.contains("haiku") {
            return false;
        }
        // auto 模型由后端决定，保守不传
        if lower == "auto" {
            return false;
        }
        // sonnet 4.5+, opus 4.5+ 支持
        true
    }
}

fn default_budget_tokens() -> i32 {
    20000
}
fn deserialize_budget_tokens<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = i32::deserialize(deserializer)?;
    Ok(value.min(MAX_BUDGET_TOKENS))
}

/// OutputConfig 配置
#[derive(Debug, Deserialize, Clone)]
pub struct OutputConfig {
    #[serde(default = "default_effort")]
    pub effort: String,
    /// 结构化输出格式配置
    #[serde(default)]
    pub format: Option<OutputFormat>,
}

/// 输出格式配置（模拟结构化输出）
#[derive(Debug, Deserialize, Clone)]
pub struct OutputFormat {
    /// 格式类型，目前仅支持 "json_schema"
    #[serde(rename = "type")]
    pub format_type: String,
    /// JSON Schema 定义
    #[serde(default)]
    pub schema: Option<serde_json::Value>,
    /// Schema 名称（可选）
    #[serde(default)]
    #[allow(dead_code)]
    pub name: Option<String>,
}

impl OutputConfig {
    /// 归一化 effort 值：仅接受 low/medium/high/xhigh/max，非法值回退 high
    pub fn normalized_effort(&self) -> &str {
        match self.effort.as_str() {
            "low" | "medium" | "high" | "xhigh" | "max" => self.effort.as_str(),
            _ => {
                tracing::warn!("未知的 thinking effort 值 '{}', 回退为 'high'", self.effort);
                "high"
            }
        }
    }

    /// 是否启用了结构化输出
    pub fn has_structured_output(&self) -> bool {
        self.format
            .as_ref()
            .is_some_and(|f| f.format_type == "json_schema" && f.schema.is_some())
    }
}

fn default_effort() -> String {
    "high".to_string()
}

/// Claude Code 请求中的 metadata
#[derive(Debug, Clone, Deserialize)]
pub struct Metadata {
    /// 用户 ID，格式如: user_xxx_account__session_0b4445e1-f5be-49e1-87ce-62bbc28ad705
    pub user_id: Option<String>,
}

/// Messages 请求体
#[derive(Debug, Clone, Deserialize)]
pub struct MessagesRequest {
    pub model: String,
    /// 为 Anthropic API 兼容保留，实际不透传给 Kiro 上游
    pub max_tokens: i32,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default, deserialize_with = "deserialize_system")]
    pub system: Option<Vec<SystemMessage>>,
    pub tools: Option<Vec<Tool>>,
    #[allow(dead_code)]
    pub tool_choice: Option<serde_json::Value>,
    pub thinking: Option<Thinking>,
    pub output_config: Option<OutputConfig>,
    /// Claude Code 请求中的 metadata，包含 session 信息
    pub metadata: Option<Metadata>,
}

/// 反序列化 system 字段，支持字符串或数组格式
fn deserialize_system<'de, D>(deserializer: D) -> Result<Option<Vec<SystemMessage>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // 创建一个 visitor 来处理 string 或 array
    struct SystemVisitor;

    impl<'de> serde::de::Visitor<'de> for SystemVisitor {
        type Value = Option<Vec<SystemMessage>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or an array of system messages")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(vec![SystemMessage {
                text: value.to_string(),
                block_type: None,
                cache_control: None,
            }]))
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut messages = Vec::new();
            while let Some(msg) = seq.next_element()? {
                messages.push(msg);
            }
            Ok(if messages.is_empty() {
                None
            } else {
                Some(messages)
            })
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            serde::de::Deserialize::deserialize(deserializer)
        }
    }

    deserializer.deserialize_any(SystemVisitor)
}

/// 消息
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Message {
    pub role: String,
    /// 可以是 string 或 ContentBlock 数组
    pub content: serde_json::Value,
}

/// 系统消息
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SystemMessage {
    pub text: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub block_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

/// 工具定义
///
/// 支持两种格式：
/// 1. 普通工具：{ name, description, input_schema }
/// 2. WebSearch 工具：{ type: "web_search_20250305", name: "web_search", max_uses: 8 }
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Tool {
    /// 工具类型，如 "web_search_20250305"（可选，仅 WebSearch 工具）
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub tool_type: Option<String>,
    /// 工具名称
    #[serde(default)]
    pub name: String,
    /// 工具描述（普通工具必需，WebSearch 工具可选）
    #[serde(default)]
    pub description: String,
    /// 输入参数 schema（普通工具必需，WebSearch 工具无此字段）
    #[serde(default)]
    pub input_schema: HashMap<String, serde_json::Value>,
    /// 最大使用次数（仅 WebSearch 工具）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<i32>,
    /// 缓存控制
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

impl Tool {
    /// 检查是否为 WebSearch 工具
    #[allow(dead_code)]
    pub fn is_web_search(&self) -> bool {
        self.tool_type
            .as_ref()
            .is_some_and(|t| t.starts_with("web_search"))
    }
}

/// 内容块
#[derive(Debug, Deserialize, Serialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ImageSource>,
    /// 文档标题（document 类型专用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 文档上下文提示（document 类型专用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// 图片/文档数据源
#[derive(Debug, Deserialize, Serialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    #[serde(default)]
    pub media_type: String,
    #[serde(default)]
    pub data: String,
    /// URL image source (not supported by this proxy)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

// === Count Tokens 端点类型 ===

/// Token 计数请求
#[derive(Debug, Serialize, Deserialize)]
pub struct CountTokensRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_system"
    )]
    pub system: Option<Vec<SystemMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
}

/// Token 计数响应
#[derive(Debug, Serialize, Deserialize)]
pub struct CountTokensResponse {
    pub input_tokens: i32,
}

/// 根据模型名称获取上下文窗口大小
///
/// - Opus 4.6 和 Sonnet 4.6 系列: 1,000,000 tokens
/// - 其他模型: 200,000 tokens
pub fn get_context_window_size(model: &str) -> i32 {
    let model_lower = model.to_lowercase();
    if (model_lower.contains("opus") || model_lower.contains("sonnet"))
        && (model_lower.contains("4-6") || model_lower.contains("4.6"))
    {
        1_000_000
    } else {
        200_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_unsupported_fields_silently() {
        let json = r#"{
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "stream": false,
            "messages": [{"role": "user", "content": "hello"}],
            "temperature": 0.5,
            "top_p": 0.9,
            "top_k": 50,
            "stop_sequences": ["X"],
            "cache_control": {"type": "ephemeral"},
            "container": "abc",
            "service_tier": "auto",
            "inference_geo": "us",
            "mcp_servers": [],
            "thinking": {"type": "enabled", "budget_tokens": 10000, "display": "summarized"},
            "output_config": {"effort": "high", "task_budget": {"some": 123}}
        }"#;

        let req: MessagesRequest =
            serde_json::from_str(json).expect("should silently drop unsupported fields");

        assert_eq!(req.model, "claude-sonnet-4-20250514");
        assert_eq!(req.max_tokens, 1024);
        assert!(!req.stream);
        assert_eq!(req.messages.len(), 1);
        assert!(req.tools.is_none());
        assert!(req.tool_choice.is_none());
        assert!(req.metadata.is_none());

        let thinking = req.thinking.expect("thinking should be present");
        assert_eq!(thinking.thinking_type, "enabled");
        assert_eq!(thinking.budget_tokens, 10000);

        let output_config = req.output_config.expect("output_config should be present");
        assert_eq!(output_config.effort, "high");
        assert!(output_config.format.is_none());
    }

    #[test]
    fn error_response_serializes_in_anthropic_envelope() {
        let err = ErrorResponse::without_request_id("invalid_request_error", "test message");
        let body = serde_json::to_value(&err).expect("should serialize");

        assert_eq!(body["type"], "error");
        assert!(body["request_id"].as_str().unwrap_or("").starts_with("req_"));
        assert!(!body["request_id"].as_str().unwrap().is_empty());
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["message"], "test message");
    }
}
