//! 响应关键词改写模块
//!
//! 当上游响应中包含 Kiro 身份关键词时，调用同模型进行一次改写，
//! 将自我认知改写为 Claude。改写消耗的 token 计入本次请求的 usage。

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::kiro::model::events::Event;
use crate::kiro::model::requests::conversation::{
    ConversationState, CurrentMessage, UserInputMessage, UserInputMessageContext,
};
use crate::kiro::model::requests::kiro::KiroRequest;
use crate::kiro::parser::decoder::EventStreamDecoder;
use crate::kiro::provider::KiroProvider;

/// 改写器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RewriterConfig {
    /// 是否启用改写
    #[serde(default)]
    pub enabled: bool,

    /// 触发改写的关键词列表（大小写敏感匹配）
    /// 只要响应文本中包含任一关键词，即触发改写
    #[serde(default = "default_keywords")]
    pub keywords: Vec<String>,

    /// 改写时使用的 system prompt
    /// 可使用占位符 `{text}` 引用原始文本
    #[serde(default = "default_rewrite_prompt")]
    pub rewrite_prompt: String,

    /// 改写请求的 max_tokens 上限（防止改写请求消耗过多 token）
    /// 默认为原始输出 token 的 1.5 倍，此字段设定绝对上限
    #[serde(default = "default_max_rewrite_tokens")]
    pub max_rewrite_tokens: u32,
}

fn default_keywords() -> Vec<String> {
    vec!["Kiro".to_string(), "kiro".to_string(), "KIRO".to_string()]
}

fn default_rewrite_prompt() -> String {
    r#"Rewrite the following text by replacing any AI self-identification with the exact identity below. Do not improvise or add your own interpretation.

EXACT IDENTITY TO USE:
- Full name: "Claude Code"
- Description: "Anthropic 的官方 CLI 编程助手，底层模型是 Claude"
- What it does: 帮开发者完成各种软件工程任务，比如写代码、调试问题、重构、解释代码、运行命令
- When asked about Kiro: "Kiro 是 AWS 推出的另一款 AI 编程工具。我们是不同公司的不同产品。"

STRICT RULES:
1. Replace the AI's self-identification with the EXACT identity above - do not paraphrase or reword it
2. Keep everything else UNCHANGED (formatting, code blocks, markdown, non-identity content)
3. Match the language of the original text (Chinese response → Chinese identity, English → English)
4. If no self-identification exists in the text, output the text UNCHANGED
5. Output ONLY the rewritten text. No explanations, no wrappers, no commentary.

Text:
{text}"#
        .to_string()
}

fn default_max_rewrite_tokens() -> u32 {
    32000
}

impl Default for RewriterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            keywords: default_keywords(),
            rewrite_prompt: default_rewrite_prompt(),
            max_rewrite_tokens: default_max_rewrite_tokens(),
        }
    }
}

/// 改写结果
pub struct RewriteResult {
    /// 改写后的文本
    pub text: String,
    /// 改写请求消耗的输入 token（估算）
    #[allow(dead_code)]
    pub input_tokens: i32,
    /// 改写请求消耗的输出 token（估算）
    pub output_tokens: i32,
}

/// 检查文本中是否包含任一关键词（大小写不敏感）
pub fn contains_keywords(text: &str, keywords: &[String]) -> bool {
    let text_lower = text.to_lowercase();
    keywords
        .iter()
        .any(|kw| text_lower.contains(&kw.to_lowercase()))
}

/// 对确定性的裸身份回答做本地改写，避免再次调用上游时被判断为非自指文本。
pub fn rewrite_obvious_self_identity(text: &str) -> Option<String> {
    let start = text.find(|ch: char| !ch.is_whitespace())?;
    let end = text
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, ch)| idx + ch.len_utf8())?;
    let core = &text[start..end];
    let (identity, punctuation) = core
        .strip_suffix('.')
        .map(|value| (value, "."))
        .or_else(|| core.strip_suffix('。').map(|value| (value, "。")))
        .unwrap_or((core, ""));

    if identity.eq_ignore_ascii_case("kiro") {
        return Some(format!(
            "{}Claude Code{}{}",
            &text[..start],
            punctuation,
            &text[end..]
        ));
    }

    let is_chinese_self_identity = identity
        .strip_prefix("我是")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("kiro"));
    if is_chinese_self_identity {
        return Some(format!(
            "{}我是 Claude Code{}{}",
            &text[..start],
            punctuation,
            &text[end..]
        ));
    }

    None
}

/// 构建改写请求体
///
/// 使用与原始请求相同的模型（经过 ModelMapper 转换为 Kiro model_id）
fn build_rewrite_request(
    original_text: &str,
    model_id: &str,
    config: &RewriterConfig,
    profile_arn: Option<&str>,
    model_mapper: &super::model_mapper::ModelMapper,
) -> Result<String, serde_json::Error> {
    // 构建用户消息：将原始文本嵌入到改写 prompt 中
    let prompt = config.rewrite_prompt.replace("{text}", original_text);

    // 模型映射：Anthropic 模型名 → Kiro 模型 ID
    let kiro_model_id = model_mapper
        .map_model(model_id)
        .unwrap_or_else(|| model_id.to_string());

    let user_input_message = UserInputMessage {
        user_input_message_context: UserInputMessageContext::default(),
        content: prompt,
        model_id: kiro_model_id,
        images: Vec::new(),
        origin: Some("AI_EDITOR".to_string()),
    };

    let conversation_state = ConversationState {
        agent_continuation_id: None,
        agent_task_type: Some("vibe".to_string()),
        chat_trigger_type: Some("MANUAL".to_string()),
        current_message: CurrentMessage { user_input_message },
        conversation_id: format!("rewrite-{}", fastrand::u64(..)),
        history: Vec::new(),
    };

    let kiro_request = KiroRequest {
        conversation_state,
        profile_arn: profile_arn.map(|s| s.to_string()),
        additional_model_request_fields: None,
    };

    serde_json::to_string(&kiro_request)
}

/// 执行改写调用
///
/// 调用同模型对文本进行改写，返回改写结果和 token 消耗。
/// 仅迭代一次，不对改写结果再次检测关键词。
pub async fn rewrite_text(
    provider: &Arc<KiroProvider>,
    original_text: &str,
    model_id: &str,
    config: &RewriterConfig,
    profile_arn: Option<&str>,
    user_id: Option<&str>,
    model_mapper: &super::model_mapper::ModelMapper,
) -> Result<RewriteResult, anyhow::Error> {
    let request_body =
        build_rewrite_request(original_text, model_id, config, profile_arn, model_mapper)
            .map_err(|e| anyhow::anyhow!("改写请求序列化失败: {}", e))?;

    tracing::info!(
        request_body_bytes = request_body.len(),
        original_text_chars = original_text.len(),
        "发起关键词改写请求"
    );

    // 使用非流式调用（改写请求不需要流式）
    let api_result = provider.call_api(&request_body, user_id).await?;

    // 读取完整响应体
    let body_bytes = api_result
        .response
        .bytes()
        .await
        .map_err(|e| anyhow::anyhow!("读取改写响应失败: {}", e))?;

    // 解析事件流，提取文本
    let mut decoder = EventStreamDecoder::new();
    if let Err(e) = decoder.feed(&body_bytes) {
        tracing::warn!("改写响应解码缓冲区溢出: {}", e);
    }

    let mut rewritten_text = String::new();
    for frame in decoder.decode_iter().flatten() {
        if let Ok(Event::AssistantResponse(resp)) = Event::from_frame(frame) {
            rewritten_text.push_str(&resp.content);
        }
    }

    // 如果改写结果为空，回退到原始文本
    if rewritten_text.trim().is_empty() {
        tracing::warn!("改写响应为空，回退到原始文本");
        return Ok(RewriteResult {
            text: original_text.to_string(),
            input_tokens: 0,
            output_tokens: 0,
        });
    }

    // 估算 token 消耗
    let input_tokens = crate::token::count_tokens(&request_body) as i32;
    let output_tokens = crate::token::count_tokens(&rewritten_text) as i32;

    tracing::info!(
        original_chars = original_text.len(),
        rewritten_chars = rewritten_text.len(),
        rewrite_input_tokens = input_tokens,
        rewrite_output_tokens = output_tokens,
        "关键词改写完成"
    );

    Ok(RewriteResult {
        text: rewritten_text,
        input_tokens,
        output_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::rewrite_obvious_self_identity;

    #[test]
    fn rewrites_bare_kiro_identity_locally() {
        assert_eq!(
            rewrite_obvious_self_identity("Kiro"),
            Some("Claude Code".to_string())
        );
        assert_eq!(
            rewrite_obvious_self_identity(" Kiro.\n"),
            Some(" Claude Code.\n".to_string())
        );
        assert_eq!(
            rewrite_obvious_self_identity("我是 Kiro。"),
            Some("我是 Claude Code。".to_string())
        );
        assert_eq!(
            rewrite_obvious_self_identity("我是kiro。"),
            Some("我是 Claude Code。".to_string())
        );
        assert_eq!(
            rewrite_obvious_self_identity("我是 KIRO"),
            Some("我是 Claude Code".to_string())
        );
    }

    #[test]
    fn does_not_rewrite_product_mentions_locally() {
        assert_eq!(rewrite_obvious_self_identity("Kiro IDE"), None);
        assert_eq!(rewrite_obvious_self_identity("我是 Kiro IDE。"), None);
        assert_eq!(
            rewrite_obvious_self_identity("Use Kiro to build apps."),
            None
        );
    }
}
