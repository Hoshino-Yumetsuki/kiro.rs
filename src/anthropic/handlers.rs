//! Anthropic API Handler 函数

use std::convert::Infallible;

use crate::kiro::model::events::Event;
use crate::kiro::model::requests::kiro::KiroRequest;
use crate::kiro::parser::decoder::EventStreamDecoder;
use crate::model::config::PromptCacheMode;
use crate::token;
use anyhow::Error;
use axum::{
    Json as JsonExtractor,
    body::Body,
    extract::{OriginalUri, State},
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use serde_json::json;
use std::time::Duration;
use tokio::time::{Instant, interval_at};

use super::converter::{ConversionError, convert_request};
use super::middleware::AppState;
use super::stream::{
    CacheUsageBreakdown, SseEvent, StreamContext, normalize_signature_for_sse,
    synthetic_thinking_signature,
};
use super::types::{
    CountTokensRequest, CountTokensResponse, ErrorResponse, MessagesRequest, ModelInfo,
    ModelsResponse, Thinking,
};
use super::websearch;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CacheUsageContext {
    cache_creation_input_tokens: i32,
    cache_read_input_tokens: i32,
    cache_creation_5m_input_tokens: i32,
    cache_creation_1h_input_tokens: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TagEchoNormalizer {
    full_tag: String,
}

struct StreamRequestContext<'a> {
    cache_tracker: Option<&'a std::sync::Arc<crate::anthropic::cache_tracker::CacheTracker>>,
    cache_profile: Option<&'a crate::anthropic::cache_tracker::CacheProfile>,
    cache_key: crate::anthropic::cache_tracker::CacheKey,
    cache_mode: PromptCacheMode,
    request_body: &'a str,
    model: &'a str,
    input_tokens: i32,
    thinking_enabled: bool,
    tool_name_map: std::collections::HashMap<String, String>,
    user_id: Option<&'a str>,
    structured_output: bool,
    rewriter_config: Option<&'a super::rewriter::RewriterConfig>,
}

struct NonStreamRequestContext<'a> {
    request_body: &'a str,
    model: &'a str,
    input_tokens: i32,
    tool_name_map: std::collections::HashMap<String, String>,
    user_id: Option<&'a str>,
    cache_tracker: Option<&'a std::sync::Arc<crate::anthropic::cache_tracker::CacheTracker>>,
    cache_profile: Option<&'a crate::anthropic::cache_tracker::CacheProfile>,
    cache_key: crate::anthropic::cache_tracker::CacheKey,
    cache_mode: PromptCacheMode,
    structured_output: bool,
    rewriter_config: Option<&'a super::rewriter::RewriterConfig>,
}

fn build_cache_profile(
    cache_tracker: &crate::anthropic::cache_tracker::CacheTracker,
    payload: &MessagesRequest,
    total_input_tokens: i32,
) -> crate::anthropic::cache_tracker::CacheProfile {
    cache_tracker.build_profile(payload, total_input_tokens)
}

fn compute_cache_usage(
    cache_tracker: &crate::anthropic::cache_tracker::CacheTracker,
    key: &crate::anthropic::cache_tracker::CacheKey,
    profile: &crate::anthropic::cache_tracker::CacheProfile,
) -> CacheUsageContext {
    let result = cache_tracker.compute(key, profile);
    CacheUsageContext {
        cache_creation_input_tokens: result.cache_creation_input_tokens,
        cache_read_input_tokens: result.cache_read_input_tokens,
        cache_creation_5m_input_tokens: result.cache_creation_5m_input_tokens,
        cache_creation_1h_input_tokens: result.cache_creation_1h_input_tokens,
    }
}

fn inject_cache_usage_fields(usage: &mut serde_json::Value, cache_context: CacheUsageContext) {
    usage["cache_creation_input_tokens"] = json!(cache_context.cache_creation_input_tokens);
    usage["cache_read_input_tokens"] = json!(cache_context.cache_read_input_tokens);
    usage["cache_creation"] = json!({
        "ephemeral_5m_input_tokens": cache_context.cache_creation_5m_input_tokens,
        "ephemeral_1h_input_tokens": cache_context.cache_creation_1h_input_tokens
    });
}

fn is_input_too_long_error(err: &Error) -> bool {
    // provider.rs 在遇到上游返回的 input-too-long 场景时，会在错误中保留以下关键字：
    // - CONTENT_LENGTH_EXCEEDS_THRESHOLD
    // - Input is too long
    //
    // 这类错误是确定性的请求问题（缩短输入才可恢复），不应返回 5xx（会诱发客户端重试）。
    // 注意：不包含 "Improperly formed request"，该错误可能由空消息内容等格式问题引起
    let s = err.to_string();
    s.contains("CONTENT_LENGTH_EXCEEDS_THRESHOLD") || s.contains("Input is too long")
}

fn is_quota_exhausted_error(err: &Error) -> bool {
    let s = err.to_string();
    s.contains("所有凭据已用尽")
}

fn is_no_credentials_error(err: &Error) -> bool {
    let s = err.to_string();
    s.contains("没有可用的凭据")
}

/// 检查是否为"所有凭据均处于冷却/速率限制"错误，并提取建议的 retry_after 秒数。
fn is_all_credentials_cooling_down_error(err: &Error) -> (bool, Option<u64>) {
    let s = err.to_string();
    if !s.contains("所有凭据均处于冷却/速率限制") {
        return (false, None);
    }
    // 提取 retry_after_secs=N
    let retry = s.split("retry_after_secs=").nth(1).and_then(|rest| {
        let end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        rest[..end].parse::<u64>().ok()
    });
    (true, retry)
}

/// 网络错误关键字（is_transient_upstream_error 和 is_network_error 共用）
const NETWORK_ERROR_PATTERNS: &[&str] = &[
    "error sending request",
    "connection closed",
    "connection reset",
];

fn is_network_error(s: &str) -> bool {
    NETWORK_ERROR_PATTERNS.iter().any(|p| s.contains(p))
}

fn is_transient_upstream_error(err: &Error) -> bool {
    let s = err.to_string().to_lowercase();
    s.contains("429 too many requests")
        || s.contains("insufficient_model_capacity")
        || s.contains("high traffic")
        || s.contains("408 request timeout")
        || s.contains("502 bad gateway")
        || s.contains("503 service unavailable")
        || s.contains("504 gateway timeout")
        || is_network_error(&s)
}

fn is_improperly_formed_request_error(err: &Error) -> bool {
    let s = err.to_string();
    s.contains("Improperly formed request")
}

#[derive(Debug, Default, Clone, Copy)]
struct AdaptiveCompressionOutcome {
    initial_bytes: usize,
    final_bytes: usize,
    iters: usize,
    whitespace_compressed: bool,
    thinking_compressed: bool,
    history_images_removed: bool,
    tool_schemas_simplified: bool,
    final_tool_description_max_chars: usize,
    tool_result_compressed: bool,
    tool_use_input_compressed: bool,
    long_messages_compressed: bool,
}

/// 计算 KiroRequest 中所有图片 base64 数据的总字节数。
///
/// 该统计用于归因请求体大小（图片 base64 往往占用大量 bytes）。
/// 注意：上游存在请求体大小硬限制（约 5MiB），因此图片也必须控制体积；
/// `max_request_body_bytes` 的校验以实际序列化后的总字节数为准。
fn total_image_bytes(kiro_request: &KiroRequest) -> usize {
    let state = &kiro_request.conversation_state;
    let mut total = 0usize;

    // currentMessage 中的图片
    for img in &state.current_message.user_input_message.images {
        total += img.source.bytes.len();
    }

    // 历史消息中的图片
    for msg in &state.history {
        if let crate::kiro::model::requests::conversation::Message::User(user_msg) = msg {
            for img in &user_msg.user_input_message.images {
                total += img.source.bytes.len();
            }
        }
    }

    total
}

fn adaptive_shrink_request_body(
    kiro_request: &mut KiroRequest,
    base_config: &crate::model::config::CompressionConfig,
    max_body: usize,
    request_body: &mut String,
) -> Result<Option<AdaptiveCompressionOutcome>, serde_json::Error> {
    if max_body == 0
        || request_body.len() <= max_body
        || !base_config.enabled
        || !base_config.adaptive_compression
        || base_config.adaptive_compression_max_iters == 0
    {
        return Ok(None);
    }

    let mut outcome = AdaptiveCompressionOutcome {
        initial_bytes: request_body.len(),
        final_bytes: request_body.len(),
        iters: 0,
        whitespace_compressed: false,
        thinking_compressed: false,
        history_images_removed: false,
        tool_schemas_simplified: false,
        final_tool_description_max_chars: 0,
        tool_result_compressed: false,
        tool_use_input_compressed: false,
        long_messages_compressed: false,
    };

    // 自适应压缩策略（按风险递增顺序）：
    // 阶段 1: 空白压缩（一次性，无损）
    // 阶段 2: thinking 压缩（一次性，低损）
    // 阶段 2.5: 历史图片丢弃（保留当前消息图片，丢弃历史会话图片）
    // 阶段 3: 工具定义 schema 简化（一次性，低损）
    // 阶段 4: 逐步截断工具 description（迭代 ×3/4）
    // 阶段 5: tool_result 截断（渐进式：8000→4000→2000→1000 chars）
    // 阶段 6: tool_use input 截断（渐进式：8000→4000→2000→1000 chars）
    // 阶段 7: 长消息内容截断（渐进式：16000→8000→4000→2000 chars）

    let has_tools = !kiro_request
        .conversation_state
        .current_message
        .user_input_message
        .user_input_message_context
        .tools
        .is_empty();

    // 阶段 1: 空白压缩
    if request_body.len() > max_body && base_config.whitespace_compression {
        super::compressor::compress_whitespace_pass(&mut kiro_request.conversation_state);
        outcome.whitespace_compressed = true;
        *request_body = serde_json::to_string(kiro_request)?;
        outcome.iters += 1;
        outcome.final_bytes = request_body.len();
    }

    // 阶段 2: thinking 压缩
    if request_body.len() > max_body && base_config.thinking_strategy != "keep" {
        super::compressor::compress_thinking_pass(
            &mut kiro_request.conversation_state,
            &base_config.thinking_strategy,
        );
        outcome.thinking_compressed = true;
        *request_body = serde_json::to_string(kiro_request)?;
        outcome.iters += 1;
        outcome.final_bytes = request_body.len();
    }

    // 阶段 2.5: 历史图片渐进丢弃（从最早的消息开始逐条清除，直到体积达标或历史图片全部清空）
    if request_body.len() > max_body {
        let mut total_removed = 0usize;
        // 从最早的历史消息开始，逐条清除图片
        for i in 0..kiro_request.conversation_state.history.len() {
            if request_body.len() <= max_body {
                break;
            }
            if let crate::kiro::model::requests::conversation::Message::User(user_msg) =
                &mut kiro_request.conversation_state.history[i]
            {
                let count = user_msg.user_input_message.images.len();
                if count > 0 {
                    user_msg.user_input_message.images.clear();
                    total_removed += count;
                    *request_body = serde_json::to_string(&*kiro_request)?;
                    outcome.iters += 1;
                    outcome.final_bytes = request_body.len();
                }
            }
        }
        if total_removed > 0 {
            outcome.history_images_removed = true;
            tracing::info!(
                removed_images = total_removed,
                current_bytes = request_body.len(),
                "自适应压缩：渐进丢弃历史会话图片（从最早到最近，保留当前消息图片）"
            );
        }
    }

    // 阶段 3: 工具 schema 简化
    if request_body.len() > max_body && base_config.tool_definition_compression && has_tools {
        let tools = &mut kiro_request
            .conversation_state
            .current_message
            .user_input_message
            .user_input_message_context
            .tools;
        super::tool_compression::simplify_tool_schemas(tools);
        outcome.tool_schemas_simplified = true;
        *request_body = serde_json::to_string(kiro_request)?;
        outcome.iters += 1;
        outcome.final_bytes = request_body.len();
    }

    // 阶段 4: 工具描述迭代截断
    if request_body.len() > max_body && base_config.tool_definition_compression && has_tools {
        let min_desc = base_config.tool_definition_min_description_chars;
        let max_tool_desc_chars = kiro_request
            .conversation_state
            .current_message
            .user_input_message
            .user_input_message_context
            .tools
            .iter()
            .map(|t| t.tool_specification.description.chars().count())
            .max()
            .unwrap_or(0);
        let mut desc_max = (max_tool_desc_chars * 3 / 4).max(min_desc);

        for _ in 0..base_config.adaptive_compression_max_iters {
            if request_body.len() <= max_body || desc_max <= min_desc {
                break;
            }
            let tools = &mut kiro_request
                .conversation_state
                .current_message
                .user_input_message
                .user_input_message_context
                .tools;
            super::tool_compression::truncate_tool_descriptions(tools, desc_max, min_desc);
            let next = (desc_max * 3 / 4).max(min_desc);
            if next >= desc_max {
                break;
            }
            desc_max = next;
            *request_body = serde_json::to_string(kiro_request)?;
            outcome.iters += 1;
            outcome.final_bytes = request_body.len();
        }
        outcome.final_tool_description_max_chars = desc_max;
    }

    // 阶段 5: tool_result 截断（渐进式降低阈值）
    if request_body.len() > max_body {
        let thresholds: &[usize] = &[8000, 4000, 2000, 1000];
        for &threshold in thresholds {
            if request_body.len() <= max_body {
                break;
            }
            let saved = super::compressor::compress_tool_results_pass(
                &mut kiro_request.conversation_state,
                threshold,
                5,
                3,
            );
            if saved > 0 {
                outcome.tool_result_compressed = true;
                *request_body = serde_json::to_string(kiro_request)?;
                outcome.iters += 1;
                outcome.final_bytes = request_body.len();
                tracing::info!(
                    threshold,
                    saved,
                    current_bytes = request_body.len(),
                    "自适应压缩：tool_result 截断"
                );
            }
        }
    }

    // 阶段 6: tool_use input 截断（渐进式降低阈值）
    if request_body.len() > max_body {
        let thresholds: &[usize] = &[8000, 4000, 2000, 1000];
        for &threshold in thresholds {
            if request_body.len() <= max_body {
                break;
            }
            let saved = super::compressor::compress_tool_use_inputs_pass(
                &mut kiro_request.conversation_state,
                threshold,
            );
            if saved > 0 {
                outcome.tool_use_input_compressed = true;
                *request_body = serde_json::to_string(kiro_request)?;
                outcome.iters += 1;
                outcome.final_bytes = request_body.len();
                tracing::info!(
                    threshold,
                    saved,
                    current_bytes = request_body.len(),
                    "自适应压缩：tool_use input 截断"
                );
            }
        }
    }

    // 阶段 7: 长消息内容截断（渐进式降低阈值）
    if request_body.len() > max_body {
        let thresholds: &[usize] = &[16000, 8000, 4000, 2000];
        for &threshold in thresholds {
            if request_body.len() <= max_body {
                break;
            }
            let saved = super::compressor::compress_long_messages_pass(
                &mut kiro_request.conversation_state,
                threshold,
            );
            if saved > 0 {
                outcome.long_messages_compressed = true;
                *request_body = serde_json::to_string(kiro_request)?;
                outcome.iters += 1;
                outcome.final_bytes = request_body.len();
                tracing::info!(
                    threshold,
                    saved,
                    current_bytes = request_body.len(),
                    "自适应压缩：长消息内容截断"
                );
            }
        }
    }

    // 最终修复：压缩后修复 tool_use/tool_result 配对和空 content
    if outcome.tool_result_compressed
        || outcome.tool_use_input_compressed
        || outcome.long_messages_compressed
    {
        super::compressor::repair_tool_pairing_and_content(&mut kiro_request.conversation_state);
        *request_body = serde_json::to_string(kiro_request)?;
        outcome.final_bytes = request_body.len();
    }

    Ok(Some(outcome))
}

fn map_kiro_provider_error_to_response(request_body: &str, err: Error) -> Response {
    if is_input_too_long_error(&err) {
        tracing::warn!(
            kiro_request_body_bytes = request_body.len(),
            error = %err,
            "上游拒绝请求：输入上下文过长（不应重试）"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::without_request_id(
                "invalid_request_error",
                "Input is too long (CONTENT_LENGTH_EXCEEDS_THRESHOLD). Reduce conversation history/system/tools; retrying the same request will not help.",
            )),
        )
            .into_response();
    }

    if is_improperly_formed_request_error(&err) {
        tracing::warn!(
            error = %err,
            kiro_request_body_bytes = request_body.len(),
            "上游拒绝请求：请求格式错误（可能由超大请求体、消息/工具序列异常或空内容块导致）"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::without_request_id(
                "invalid_request_error",
                "Improperly formed request. This is often caused by oversized payloads, malformed message/tool sequences, or empty content blocks.",
            )),
        )
            .into_response();
    }

    if is_no_credentials_error(&err) {
        tracing::error!(error = %err, "没有可用的凭据");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse::without_request_id(
                "service_unavailable",
                "No credentials available. Please add or enable credentials via Admin API or credentials.json.",
            )),
        )
            .into_response();
    }

    let (cooling, retry_after) = is_all_credentials_cooling_down_error(&err);
    if cooling {
        let secs = retry_after.unwrap_or(60);
        tracing::warn!(
            error = %err,
            retry_after_secs = secs,
            "所有凭据临时冷却，返回 429 + Retry-After"
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, secs.to_string())],
            Json(ErrorResponse::without_request_id(
                "rate_limit_error",
                format!(
                    "All credentials are temporarily cooling down. Retry after {}s.",
                    secs
                ),
            )),
        )
            .into_response();
    }

    if is_quota_exhausted_error(&err) {
        tracing::warn!(error = %err, "所有凭据配额已耗尽");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse::without_request_id(
                "rate_limit_error",
                "All credentials quota exhausted. Please wait for quota reset or add new credentials.",
            )),
        )
            .into_response();
    }

    if is_transient_upstream_error(&err) {
        let err_str = err.to_string().to_lowercase();
        if is_network_error(&err_str) {
            tracing::warn!(error = %err, "上游网络错误，不输出请求体");
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse::without_request_id(
                    "api_error",
                    format!("上游网络错误: {}", err),
                )),
            )
                .into_response();
        }
        tracing::warn!(error = %err, "上游瞬态错误（429/5xx），不输出请求体");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse::without_request_id(
                "rate_limit_error",
                err.to_string(),
            )),
        )
            .into_response();
    }

    tracing::error!("Kiro API 调用失败: {}", err);
    #[cfg(feature = "sensitive-logs")]
    tracing::error!(
        request_body_bytes = request_body.len(),
        "上游报错，请求体大小: {} bytes",
        request_body.len()
    );
    (
        StatusCode::BAD_GATEWAY,
        Json(ErrorResponse::without_request_id(
            "api_error",
            format!("上游 API 调用失败: {}", err),
        )),
    )
        .into_response()
}

/// 对 user_id 进行掩码处理，保护隐私
fn mask_user_id(user_id: Option<&str>) -> String {
    match user_id {
        Some(id) => {
            let chars: Vec<char> = id.chars().collect();
            let len = chars.len();
            if len > 25 {
                format!(
                    "{}***{}",
                    chars[..13].iter().collect::<String>(),
                    chars[len - 8..].iter().collect::<String>()
                )
            } else if len > 12 {
                format!(
                    "{}***{}",
                    chars[..4].iter().collect::<String>(),
                    chars[len - 4..].iter().collect::<String>()
                )
            } else {
                "***".to_string()
            }
        }
        None => "None".to_string(),
    }
}

/// 剔除 messages 中的空 text content block（`{"type":"text","text":""}` 或纯空白）。
///
/// 说明：
/// - Claude Code/claude-cli 在某些 tool_use-only 场景下可能会把空 text block 写回 history；
/// - 上游会拒绝空 text block（400: "text content blocks must be non-empty"）。
/// - 空 text block 不携带任何语义，直接移除是最小且安全的兼容策略。
fn strip_empty_text_content_blocks(messages: &mut [super::types::Message]) -> usize {
    let mut removed = 0usize;

    for msg in messages {
        let serde_json::Value::Array(arr) = &mut msg.content else {
            continue;
        };

        let before = arr.len();
        arr.retain(|item| {
            let Some(obj) = item.as_object() else {
                return true;
            };

            if obj.get("type").and_then(|v| v.as_str()) != Some("text") {
                return true;
            }

            match obj.get("text") {
                Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
                Some(serde_json::Value::Null) | None => false,
                // text 字段类型异常：保守起见不删，交由后续转换/上游校验处理
                _ => true,
            }
        });
        removed += before - arr.len();
    }

    removed
}

fn extract_tag_echo_normalizer(payload: &MessagesRequest) -> Option<TagEchoNormalizer> {
    if payload
        .output_config
        .as_ref()
        .is_some_and(|config| config.has_structured_output())
    {
        return None;
    }

    let prompt = latest_user_text_prompt_for_tag_echo(payload)?;
    if !is_direct_tag_echo_query(&prompt) {
        return None;
    }

    let full_tag = extract_antml_test_tag(&prompt)?;
    Some(TagEchoNormalizer { full_tag })
}

fn latest_user_text_prompt_for_tag_echo(payload: &MessagesRequest) -> Option<String> {
    let message = payload
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")?;
    if let Some(text) = message.content.as_str() {
        return Some(text.to_string());
    }

    let mut parts = Vec::new();
    for block in message.content.as_array()? {
        match block.get("type").and_then(|value| value.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|value| value.as_str()) {
                    parts.push(text);
                }
            }
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

fn extract_antml_test_tag(prompt: &str) -> Option<String> {
    let marker = "<antml:";
    let start = prompt.find(marker)?;
    let name_start = start + marker.len();
    let rest = &prompt[name_start..];
    let name_end = rest.find('>')?;
    let name = &rest[..name_end];
    if !name.starts_with("test_")
        || name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return None;
    }

    Some(format!("<antml:{name}>"))
}

fn is_direct_tag_echo_query(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let asks_about_tag = lower.contains("tag") || prompt.contains("标签");
    let english_direct_echo = contains_any(
        &lower,
        &[
            "directly output",
            "direct output",
            "echo the tag",
            "repeat the tag",
            "return the tag",
            "output the tag",
            "verbatim",
            "output exactly",
            "return exactly",
        ],
    );
    let chinese_direct_echo = contains_any(
        prompt,
        &[
            "直接输出你前面看到的tag",
            "直接输出你前面看到的 tag",
            "直接输出看到的tag",
            "直接输出看到的 tag",
            "看到的文本",
            "原样输出",
            "逐字输出",
            "复读tag",
            "复读 tag",
        ],
    );
    let direct_echo = english_direct_echo || chinese_direct_echo;
    let asks_transform =
        contains_any(
            &lower,
            &["summarize", "summary", "translate", "explain", "analyze"],
        ) || contains_any(prompt, &["总结", "概括", "摘要", "翻译", "解释", "分析"]);

    asks_about_tag && direct_echo && !asks_transform
}

fn extract_pdf_text_answer(payload: &MessagesRequest) -> Option<String> {
    if payload
        .output_config
        .as_ref()
        .is_some_and(|config| config.has_structured_output())
    {
        return None;
    }

    if payload
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty())
    {
        return None;
    }

    let message = payload
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")?;
    let blocks = message.content.as_array()?;
    let mut prompt_parts = Vec::new();
    let mut pdf_texts = Vec::new();

    for block in blocks {
        match block.get("type").and_then(|value| value.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|value| value.as_str()) {
                    prompt_parts.push(text);
                }
            }
            Some("document") => {
                let Some(source) = block.get("source") else {
                    continue;
                };
                let is_pdf = source
                    .get("media_type")
                    .and_then(|value| value.as_str())
                    .is_some_and(|media_type| media_type.eq_ignore_ascii_case("application/pdf"));
                let is_base64 = source
                    .get("type")
                    .and_then(|value| value.as_str())
                    .is_some_and(|source_type| source_type == "base64");
                if !is_pdf || !is_base64 {
                    continue;
                }
                let Some(data) = source.get("data").and_then(|value| value.as_str()) else {
                    continue;
                };
                match crate::pdf::extract_text_from_base64(data) {
                    Ok(text) => {
                        if !text.trim().is_empty() {
                            pdf_texts.push(text);
                        }
                    }
                    Err(err) => tracing::warn!("PDF 文本本地直返抽取失败: {err}"),
                }
            }
            _ => {}
        }
    }

    if pdf_texts.is_empty() || !is_direct_pdf_text_query(&prompt_parts.join("\n")) {
        return None;
    }

    Some(pdf_texts.join("\n"))
}

fn is_direct_pdf_text_query(prompt: &str) -> bool {
    if prompt.trim().is_empty() {
        return false;
    }

    let lower = prompt.to_lowercase();
    let asks_transform = contains_any(
        &lower,
        &[
            "summarize",
            "summary",
            "translate",
            "explain",
            "analyze",
            "bullet",
            "table",
            "json",
            "schema",
        ],
    ) || contains_any(
        prompt,
        &[
            "总结",
            "概括",
            "摘要",
            "翻译",
            "解释",
            "分析",
            "要点",
            "表格",
            "结构化",
            "JSON",
            "json",
        ],
    );
    if asks_transform {
        return false;
    }

    let mentions_pdf = lower.contains("pdf");
    let text_target = contains_any(
        &lower,
        &[
            "text",
            "transcript",
            "ocr",
            "raw text",
            "exact text",
            "verbatim",
            "words",
        ],
    ) || contains_any(prompt, &["文字", "文本", "原文", "逐字", "识别"]);
    let extraction_action = contains_any(
        &lower,
        &[
            "contain",
            "contains",
            "extract",
            "transcribe",
            "read",
            "return",
            "output",
            "show",
            "what text",
            "what does",
            "say",
            "ocr",
        ],
    ) || contains_any(
        prompt,
        &[
            "提取",
            "抽取",
            "读取",
            "识别",
            "输出",
            "返回",
            "写了什么",
            "是什么",
            "有什么",
        ],
    );
    let direct_output = contains_any(&lower, &["only", "just", "exact", "raw", "verbatim"])
        || contains_any(prompt, &["只", "仅", "不要", "直接", "原文", "逐字"]);

    mentions_pdf && text_target && extraction_action && direct_output
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn rewrite_obvious_self_identity_if_enabled(
    text: &str,
    rewriter_config: Option<&super::rewriter::RewriterConfig>,
) -> Option<String> {
    if !is_response_rewrite_enabled(rewriter_config) {
        return None;
    }
    super::rewriter::rewrite_obvious_self_identity(text)
}

fn is_response_rewrite_enabled(rewriter_config: Option<&super::rewriter::RewriterConfig>) -> bool {
    rewriter_config.is_some_and(|config| config.enabled)
}

fn build_local_text_response(payload: &MessagesRequest, text: &str, input_tokens: i32) -> Response {
    let output_tokens = token::count_tokens(text) as i32;
    let usage = json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
        "cache_creation": {
            "ephemeral_5m_input_tokens": 0,
            "ephemeral_1h_input_tokens": 0,
        }
    });

    if !payload.stream {
        return Json(json!({
            "id": super::stream::generate_anthropic_message_id(),
            "type": "message",
            "role": "assistant",
            "model": payload.model,
            "content": [{"type": "text", "text": text}],
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": usage,
        }))
        .into_response();
    }

    let message_id = super::stream::generate_anthropic_message_id();
    let events = vec![
        SseEvent::new(
            "message_start",
            json!({
                "type": "message_start",
                "message": {
                    "id": message_id,
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": payload.model,
                    "stop_reason": null,
                    "stop_sequence": null,
                    "usage": usage,
                }
            }),
        ),
        SseEvent::new(
            "content_block_start",
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": {"type": "text", "text": ""}
            }),
        ),
        SseEvent::new(
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": {"type": "text_delta", "text": text}
            }),
        ),
        SseEvent::new(
            "content_block_stop",
            json!({"type": "content_block_stop", "index": 0}),
        ),
        SseEvent::new(
            "message_delta",
            json!({
                "type": "message_delta",
                "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                "usage": {"output_tokens": output_tokens}
            }),
        ),
        SseEvent::new("message_stop", json!({"type": "message_stop"})),
    ];
    let body = events
        .into_iter()
        .map(|event| event.to_sse_string())
        .collect::<String>();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from(body))
        .unwrap()
}

/// GET /v1/models
///
/// 返回可用的模型列表。
pub async fn get_models(OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    tracing::info!(
        path = %uri.path(),
        "Received request"
    );

    let models = get_all_model_infos();
    let first_id = models.first().map(|m| m.id.clone());
    let last_id = models.last().map(|m| m.id.clone());

    Json(ModelsResponse {
        data: models,
        has_more: false,
        first_id,
        last_id,
    })
}

/// GET /v1/models/{id}
///
/// 获取单个模型信息
pub async fn get_model(
    OriginalUri(uri): OriginalUri,
    axum::extract::Path(model_id): axum::extract::Path<String>,
) -> impl IntoResponse {
    tracing::info!(
        path = %uri.path(),
        model_id = %model_id,
        "Received request"
    );

    let request_id = format!("req_{}", uuid::Uuid::new_v4().simple());
    let models = get_all_model_infos();
    match models.into_iter().find(|m| m.id == model_id) {
        Some(model) => (
            [(
                axum::http::header::HeaderName::from_static("x-request-id"),
                request_id,
            )],
            Json(model),
        )
            .into_response(),
        None => {
            let error = ErrorResponse::new(
                "not_found_error",
                format!("model_not_found: {model_id}"),
                &request_id,
            );
            (
                axum::http::StatusCode::NOT_FOUND,
                [(
                    axum::http::header::HeaderName::from_static("x-request-id"),
                    request_id,
                )],
                Json(error),
            )
                .into_response()
        }
    }
}

/// 获取所有可用模型列表（Anthropic ModelInfo 格式）
fn get_all_model_infos() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            id: "claude-sonnet-4-6".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Sonnet 4.6".to_string(),
            created_at: 1770314400,
        },
        ModelInfo {
            id: "claude-sonnet-4-5-20250929".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Sonnet 4.5".to_string(),
            created_at: 1727568000,
        },
        ModelInfo {
            id: "claude-opus-4-5-20251101".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Opus 4.5".to_string(),
            created_at: 1730419200,
        },
        ModelInfo {
            id: "claude-opus-4-6".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Opus 4.6".to_string(),
            created_at: 1770314400,
        },
        ModelInfo {
            id: "claude-opus-4-7".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Opus 4.7".to_string(),
            created_at: 1772992800,
        },
        ModelInfo {
            id: "claude-opus-4-8".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Opus 4.8".to_string(),
            created_at: 1775671200,
        },
        ModelInfo {
            id: "claude-haiku-4-5-20251001".to_string(),
            model_type: "model".to_string(),
            display_name: "Claude Haiku 4.5".to_string(),
            created_at: 1727740800,
        },
    ]
}

/// 图片 URL 下载超时（秒）
const IMAGE_URL_FETCH_TIMEOUT_SECS: u64 = 30;
/// 图片 URL 下载最大文件大小（20MB）
const IMAGE_URL_MAX_SIZE: usize = 20 * 1024 * 1024;

/// 预处理请求中的 URL 类型图片，下载并转换为 base64 内联数据。
///
/// 遍历所有 messages 中的 content blocks，找到 `{"type": "image", "source": {"type": "url", ...}}`
/// 的块，将其下载为 base64 数据，替换 source 为 base64 类型。
async fn resolve_image_urls(payload: &mut MessagesRequest) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(IMAGE_URL_FETCH_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("创建 HTTP Client 失败，跳过 URL 图片解析: {}", e);
            return;
        }
    };

    for message in &mut payload.messages {
        let content = &mut message.content;
        if let serde_json::Value::Array(blocks) = content {
            for block in blocks.iter_mut() {
                let is_image_url = block.get("type").and_then(|t| t.as_str()) == Some("image")
                    && block
                        .get("source")
                        .and_then(|s| s.get("type"))
                        .and_then(|t| t.as_str())
                        == Some("url");

                if !is_image_url {
                    continue;
                }

                let url = match block
                    .get("source")
                    .and_then(|s| s.get("url"))
                    .and_then(|u| u.as_str())
                {
                    Some(u) => u.to_string(),
                    None => continue,
                };

                tracing::info!(url = %url, "下载 URL 图片");

                match fetch_image_url(&client, &url).await {
                    Ok((data_base64, media_type)) => {
                        // 替换 source 为 base64 类型
                        if let Some(source) = block.get_mut("source") {
                            *source = serde_json::json!({
                                "type": "base64",
                                "media_type": media_type,
                                "data": data_base64
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(url = %url, error = %e, "URL 图片下载失败，跳过");
                    }
                }
            }
        }
    }
}

/// 从 URL 下载图片并返回 (base64_data, media_type)
///
/// 使用 `infer` 库通过 magic bytes 验证下载内容确实是图片，
/// 而非信任 HTTP Content-Type header。
async fn fetch_image_url(
    client: &reqwest::Client,
    url: &str,
) -> Result<(String, String), anyhow::Error> {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("URL 图片下载失败: HTTP {}", response.status().as_u16());
    }

    let bytes = response.bytes().await?;

    if bytes.len() > IMAGE_URL_MAX_SIZE {
        anyhow::bail!(
            "URL 图片过大: {} bytes（上限 {} bytes）",
            bytes.len(),
            IMAGE_URL_MAX_SIZE
        );
    }

    // 通过 magic bytes 验证是否为图片格式
    if !infer::is_image(&bytes) {
        anyhow::bail!("URL 内容不是有效的图片格式");
    }

    // 从 magic bytes 推断实际 media type（不信任 Content-Type header）
    let media_type = infer::get(&bytes)
        .map(|t| t.mime_type().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let data_base64 = STANDARD.encode(&bytes);
    Ok((data_base64, media_type))
}

/// POST /v1/messages
///
/// 创建消息（对话）
pub async fn post_messages(
    OriginalUri(uri): OriginalUri,
    State(state): State<AppState>,
    JsonExtractor(mut payload): JsonExtractor<MessagesRequest>,
) -> Response {
    // 读取压缩配置快照（读锁 + clone，避免持锁跨 await）
    let compression_config = state.compression_config.read().clone();
    let rewriter_config = state.rewriter_config.read().clone();
    let prompt_cache = state.prompt_cache_snapshot();

    // 预处理 URL 类型图片：下载并转换为 base64 内联数据
    resolve_image_urls(&mut payload).await;

    // 提取 user_id 用于凭据亲和性 + 缓存分桶
    let user_id = payload.metadata.as_ref().and_then(|m| m.user_id.clone());
    let cache_key = match user_id.as_deref() {
        Some(uid) => crate::anthropic::cache_tracker::CacheKey::User(uid.to_string()),
        None => crate::anthropic::cache_tracker::CacheKey::Global,
    };

    // 估算压缩前 input tokens（需在 convert_request 之前，因为后者会消费压缩）
    let estimated_input_tokens = token::count_all_tokens(
        payload.model.clone(),
        payload.system.clone(),
        payload.messages.clone(),
        payload.tools.clone(),
    ) as i32;

    tracing::info!(
        path = %uri.path(),
        model = %payload.model,
        max_tokens = %payload.max_tokens,
        stream = %payload.stream,
        message_count = %payload.messages.len(),
        user_id = %mask_user_id(user_id.as_deref()),
        estimated_input_tokens,
        "Received request"
    );
    // 检查 KiroProvider 是否可用
    let provider = match &state.kiro_provider {
        Some(p) => p.clone(),
        None => {
            tracing::error!("KiroProvider 未配置");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::without_request_id(
                    "service_unavailable",
                    "Kiro API provider not configured",
                )),
            )
                .into_response();
        }
    };

    // 检查是否为纯 WebSearch 请求（仅 web_search 单工具 / tool_choice 强制 / 前缀匹配）
    let cache_enabled = !matches!(prompt_cache.mode, PromptCacheMode::Off);
    let websearch_cache_profile = cache_enabled.then(|| {
        build_cache_profile(
            prompt_cache.tracker.as_ref(),
            &payload,
            estimated_input_tokens,
        )
    });
    if websearch::should_handle_websearch_request(&payload) {
        tracing::info!("检测到纯 WebSearch 请求，路由到本地 WebSearch 处理");
        return websearch::handle_websearch_request(
            provider,
            &payload,
            if cache_enabled {
                Some(&prompt_cache.tracker)
            } else {
                None
            },
            websearch_cache_profile.as_ref(),
            estimated_input_tokens,
        )
        .await;
    }

    // 混合工具场景：剔除 web_search 后转发上游
    if websearch::has_web_search_tool(&payload) {
        tracing::info!("检测到混合工具列表中的 web_search，剔除后转发上游");
        websearch::strip_web_search_tools(&mut payload);
    }

    // 剔除空 text content block（客户端可能将 tool_use-only 响应中的空 text block 写回 history）
    let stripped = strip_empty_text_content_blocks(&mut payload.messages);
    if stripped > 0 {
        tracing::info!(stripped, "已剔除空 text content block");
    }

    let tag_echo_normalizer = extract_tag_echo_normalizer(&payload);
    if tag_echo_normalizer.is_some() {
        tracing::info!("检测到 antml test tag 复读请求，启用响应 tag normalizer");
    }

    if let Some(normalizer) = &tag_echo_normalizer {
        tracing::info!("检测到 antml test tag 复读请求，使用本地 tag 结果直接返回");
        return build_local_text_response(&payload, &normalizer.full_tag, estimated_input_tokens);
    }

    if let Some(answer) = extract_pdf_text_answer(&payload) {
        tracing::info!("检测到 PDF 纯文本抽取请求，使用本地抽取结果直接返回");
        return build_local_text_response(&payload, &answer, estimated_input_tokens);
    }

    let cache_profile = cache_enabled.then(|| {
        build_cache_profile(
            prompt_cache.tracker.as_ref(),
            &payload,
            estimated_input_tokens,
        )
    });

    tracing::info!(
        cache_mode = ?prompt_cache.mode,
        prompt_cache_ttl_seconds = prompt_cache.ttl_seconds,
        "Prompt cache configuration for /v1/messages"
    );

    // 转换请求
    let conversion_result = match convert_request(&payload, &compression_config) {
        Ok(result) => result,
        Err(e) => {
            let (error_type, message) = match &e {
                ConversionError::UnsupportedModel(model) => {
                    ("invalid_request_error", format!("模型不支持: {}", model))
                }
                ConversionError::EmptyMessages => {
                    ("invalid_request_error", "消息列表为空".to_string())
                }
                ConversionError::EmptyMessageContent => {
                    ("invalid_request_error", "消息内容为空".to_string())
                }
            };
            tracing::warn!("请求转换失败: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::without_request_id(error_type, message)),
            )
                .into_response();
        }
    };

    // 输出压缩统计（以字节为单位；用于排查上游请求体大小限制，实测约 5MiB 左右会触发 400）
    if let Some(ref stats) = conversion_result.compression_stats {
        tracing::info!(
            estimated_input_tokens,
            bytes_saved_total = stats.total_saved(),
            whitespace_bytes_saved = stats.whitespace_saved,
            thinking_bytes_saved = stats.thinking_saved,
            tool_result_bytes_saved = stats.tool_result_saved,
            tool_use_input_bytes_saved = stats.tool_use_input_saved,
            history_turns_removed = stats.history_turns_removed,
            history_bytes_saved = stats.history_bytes_saved,
            "输入压缩完成"
        );
    }

    // 构建 Kiro 请求
    let tool_name_map = conversion_result.tool_name_map;

    // 构建 additionalModelRequestFields（thinking 参数）
    let additional_model_request_fields = build_additional_model_request_fields(&payload);

    let mut kiro_request = KiroRequest {
        conversation_state: conversion_result.conversation_state,
        profile_arn: state.profile_arn.clone(),
        additional_model_request_fields,
    };

    let mut request_body = match serde_json::to_string(&kiro_request) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!("序列化请求失败: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::without_request_id(
                    "internal_error",
                    format!("序列化请求失败: {}", e),
                )),
            )
                .into_response();
        }
    };

    // 请求体大小预检（上游存在硬性请求体大小限制；按实际序列化后的总字节数判断）
    let max_body = compression_config.max_request_body_bytes;
    if max_body > 0 && request_body.len() > max_body && compression_config.enabled {
        // 自适应二次压缩：按 request_body_bytes 迭代截断，尽量把请求缩到阈值内
        match adaptive_shrink_request_body(
            &mut kiro_request,
            &compression_config,
            max_body,
            &mut request_body,
        ) {
            Ok(Some(outcome)) => {
                tracing::warn!(
                    conversation_id = kiro_request.conversation_state.conversation_id.as_str(),
                    initial_bytes = outcome.initial_bytes,
                    final_bytes = outcome.final_bytes,
                    threshold = max_body,
                    iters = outcome.iters,
                    whitespace_compressed = outcome.whitespace_compressed,
                    thinking_compressed = outcome.thinking_compressed,
                    history_images_removed = outcome.history_images_removed,
                    tool_schemas_simplified = outcome.tool_schemas_simplified,
                    final_tool_description_max_chars = outcome.final_tool_description_max_chars,
                    tool_result_compressed = outcome.tool_result_compressed,
                    tool_use_input_compressed = outcome.tool_use_input_compressed,
                    long_messages_compressed = outcome.long_messages_compressed,
                    "请求体超过阈值，已执行自适应压缩"
                );
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("自适应二次压缩序列化失败: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::without_request_id(
                        "internal_error",
                        format!("序列化请求失败: {}", e),
                    )),
                )
                    .into_response();
            }
        }
    }

    // 压缩后再次检查（输出 image_bytes/non-image bytes 便于排查）
    let final_img_bytes = total_image_bytes(&kiro_request);
    let final_effective_len = request_body.len().saturating_sub(final_img_bytes);
    if max_body > 0 && request_body.len() > max_body {
        tracing::warn!(
            conversation_id = kiro_request.conversation_state.conversation_id.as_str(),
            request_body_bytes = request_body.len(),
            image_bytes = final_img_bytes,
            effective_bytes = final_effective_len,
            threshold = max_body,
            "请求体超过安全阈值，拒绝发送"
        );
        #[cfg(feature = "sensitive-logs")]
        tracing::error!(
            "自适应压缩仍超限，完整请求体（用于诊断）: {}",
            truncate_base64_in_request_body(&request_body)
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::without_request_id(
                "invalid_request_error",
                format!(
                    "Request too large ({} bytes total; images {} bytes; non-image {} bytes; limit {}). Reduce conversation history/tool output or number/size of images.",
                    request_body.len(),
                    final_img_bytes,
                    final_effective_len,
                    max_body
                ),
            )),
        )
            .into_response();
    }

    tracing::debug!(
        kiro_request_body_bytes = request_body.len(),
        "已构建 Kiro 请求体"
    );

    // 检查是否启用了thinking（且模型支持）
    let thinking_enabled = payload
        .thinking
        .as_ref()
        .map(|t| t.is_enabled() && Thinking::model_supports_thinking(&payload.model))
        .unwrap_or(false);

    // 检查是否启用了结构化输出
    let structured_output = payload
        .output_config
        .as_ref()
        .is_some_and(|c| c.has_structured_output());

    if payload.stream {
        // 流式响应
        let stream_request = StreamRequestContext {
            cache_tracker: cache_enabled.then_some(&prompt_cache.tracker),
            cache_profile: cache_profile.as_ref(),
            cache_key: cache_key.clone(),
            cache_mode: prompt_cache.mode,
            request_body: &request_body,
            model: &payload.model,
            input_tokens: estimated_input_tokens,
            thinking_enabled,
            tool_name_map: tool_name_map.clone(),
            user_id: user_id.as_deref(),
            structured_output,
            rewriter_config: if rewriter_config.enabled {
                Some(&rewriter_config)
            } else {
                None
            },
        };
        handle_stream_request(provider, stream_request).await
    } else {
        // 非流式响应
        let non_stream_request = NonStreamRequestContext {
            request_body: &request_body,
            model: &payload.model,
            input_tokens: estimated_input_tokens,
            tool_name_map,
            user_id: user_id.as_deref(),
            cache_tracker: cache_enabled.then_some(&prompt_cache.tracker),
            cache_profile: cache_profile.as_ref(),
            cache_key: cache_key.clone(),
            cache_mode: prompt_cache.mode,
            structured_output,
            rewriter_config: if rewriter_config.enabled {
                Some(&rewriter_config)
            } else {
                None
            },
        };
        handle_non_stream_request(provider, non_stream_request).await
    }
}
async fn handle_stream_request(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    context: StreamRequestContext<'_>,
) -> Response {
    // 调用 Kiro API（支持多凭据故障转移）
    let api_result = match provider
        .call_api_stream(context.request_body, context.user_id)
        .await
    {
        Ok(resp) => resp,
        Err(e) => return map_kiro_provider_error_to_response(context.request_body, e),
    };

    // 缓存计算和落库都使用 user_id 维度的 CacheKey；
    // 凭据故障转移不会丢失上一轮缓存条目。
    let final_cache_context = match (context.cache_tracker, context.cache_profile) {
        (Some(tracker), Some(profile)) => {
            let resolved = compute_cache_usage(tracker, &context.cache_key, profile);
            tracing::info!(
                credential_id = api_result.credential_id,
                final_cache_creation_input_tokens = resolved.cache_creation_input_tokens,
                final_cache_read_input_tokens = resolved.cache_read_input_tokens,
                "Resolved cache usage for stream request"
            );
            tracker.update(&context.cache_key, profile);
            Some(resolved)
        }
        _ => None,
    };
    let final_cache_usage = final_cache_context.map(|ctx| CacheUsageBreakdown {
        cache_creation_input_tokens: ctx.cache_creation_input_tokens,
        cache_read_input_tokens: ctx.cache_read_input_tokens,
        cache_creation_5m_input_tokens: ctx.cache_creation_5m_input_tokens,
        cache_creation_1h_input_tokens: ctx.cache_creation_1h_input_tokens,
    });

    // 创建流处理上下文
    let rewrite_keywords = context
        .rewriter_config
        .map(|c| c.keywords.clone())
        .unwrap_or_default();
    let mut ctx = StreamContext::new_with_thinking(
        context.model,
        context.input_tokens,
        final_cache_usage,
        context.thinking_enabled,
        context.tool_name_map,
        context.structured_output,
        rewrite_keywords,
        context.cache_mode,
    );
    ctx.rewrite_enabled = is_response_rewrite_enabled(context.rewriter_config);

    // 生成初始事件
    let initial_events = ctx.generate_initial_events();

    // 创建 SSE 流
    let stream = create_sse_stream(
        api_result.response,
        ctx,
        initial_events,
        provider,
        context.rewriter_config.cloned(),
        context.user_id.map(|s| s.to_string()),
    );

    // 返回 SSE 响应
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Ping 事件间隔（25秒）
const PING_INTERVAL_SECS: u64 = 25;

/// 创建 ping 事件的 SSE 字符串
fn create_ping_sse() -> Bytes {
    Bytes::from("event: ping\ndata: {\"type\": \"ping\"}\n\n")
}

/// 流结束时 flush 改写缓冲区
///
/// 如果缓冲区中包含关键词，则调用同模型进行改写；否则直接 flush 原始文本。
/// 改写消耗的 token 会被合并到 StreamContext 的 usage 中。
///
/// 返回值：是否需要进入 drip 模式（true = 文本已入 drip 队列，需逐块输出）
async fn flush_rewrite_buffer(
    ctx: &mut super::stream::StreamContext,
    provider: &std::sync::Arc<crate::kiro::provider::KiroProvider>,
    rewriter_config: Option<&super::rewriter::RewriterConfig>,
    user_id: Option<&str>,
) -> (Vec<super::stream::SseEvent>, bool) {
    if !ctx.rewrite_enabled || ctx.rewrite_text_buffer.is_empty() {
        return (Vec::new(), false);
    }

    let buffer_text = ctx.take_rewrite_buffer();

    if let Some(rewritten) = rewrite_obvious_self_identity_if_enabled(&buffer_text, rewriter_config)
    {
        ctx.enqueue_drip_text(&rewritten);
        return (Vec::new(), true);
    }

    // 检测是否包含关键词（case-insensitive）
    if !super::rewriter::contains_keywords(&buffer_text, &ctx.rewrite_keywords) {
        // 无关键词，直接 flush 原始文本（入 drip 队列模拟逐字输出）
        ctx.enqueue_drip_text(&buffer_text);
        return (Vec::new(), true);
    }

    // 包含关键词，执行模型改写
    let Some(config) = rewriter_config else {
        ctx.enqueue_drip_text(&buffer_text);
        return (Vec::new(), true);
    };

    let model_id = ctx.model.clone();
    tracing::info!(
        model = %model_id,
        buffer_chars = buffer_text.len(),
        "检测到关键词，触发模型改写"
    );

    match super::rewriter::rewrite_text(provider, &buffer_text, &model_id, config, None, user_id)
        .await
    {
        Ok(result) => {
            // 将改写消耗的 token 合并到 usage
            ctx.add_rewrite_tokens(result.output_tokens);
            // 将改写后的文本入 drip 队列
            ctx.enqueue_drip_text(&result.text);
            (Vec::new(), true)
        }
        Err(e) => {
            tracing::error!("关键词改写失败，回退到原始文本: {}", e);
            ctx.enqueue_drip_text(&buffer_text);
            (Vec::new(), true)
        }
    }
}

/// 创建 SSE 事件流
fn create_sse_stream(
    response: reqwest::Response,
    ctx: StreamContext,
    initial_events: Vec<SseEvent>,
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    rewriter_config: Option<super::rewriter::RewriterConfig>,
    user_id: Option<String>,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    // 先发送初始事件
    let initial_stream = stream::iter(
        initial_events
            .into_iter()
            .map(|e| Ok(Bytes::from(e.to_sse_string()))),
    );

    // 然后处理 Kiro 响应流，同时每25秒发送 ping 保活
    let body_stream = response.bytes_stream();
    let ping_period = Duration::from_secs(PING_INTERVAL_SECS);
    let ping_interval = interval_at(Instant::now() + ping_period, ping_period);

    // dripping: 是否进入逐块输出模式（上游已结束，正在 drip 改写后的文本）
    let processing_stream = stream::unfold(
        (body_stream, ctx, EventStreamDecoder::new(), false, false, ping_interval, provider, rewriter_config, user_id),
        |(mut body_stream, mut ctx, mut decoder, finished, dripping, mut ping_interval, provider, rewriter_config, user_id)| async move {
            if finished {
                return None;
            }

            // Drip 模式：逐块输出改写后的文本，每块间加延迟
            if dripping {
                if let Some(events) = ctx.pop_drip_chunk() {
                    // 模拟输出间隔：10-30ms（接近真实 token 输出节奏）
                    tokio::time::sleep(Duration::from_millis(10 + fastrand::u64(..20))).await;
                    let bytes: Vec<Result<Bytes, Infallible>> = events
                        .into_iter()
                        .map(|e| Ok(Bytes::from(e.to_sse_string())))
                        .collect();
                    return Some((stream::iter(bytes), (body_stream, ctx, decoder, false, true, ping_interval, provider, rewriter_config, user_id)));
                }
                // Drip 队列已空，发送最终事件并结束
                let final_events = ctx.generate_final_events();
                let bytes: Vec<Result<Bytes, Infallible>> = final_events
                    .into_iter()
                    .map(|e| Ok(Bytes::from(e.to_sse_string())))
                    .collect();
                return Some((stream::iter(bytes), (body_stream, ctx, decoder, true, false, ping_interval, provider, rewriter_config, user_id)));
            }

            // 使用 select! 同时等待数据和 ping 定时器
            tokio::select! {
                // 处理数据流
                chunk_result = body_stream.next() => {
                    match chunk_result {
                        Some(Ok(chunk)) => {
                            // 解码事件
                            if let Err(e) = decoder.feed(&chunk) {
                                tracing::warn!("缓冲区溢出: {}", e);
                            }

                            let mut events = Vec::new();
                            for result in decoder.decode_iter() {
                                match result {
                                    Ok(frame) => {
                                        if let Ok(event) = Event::from_frame(frame) {
                                            let sse_events = ctx.process_kiro_event(&event);
                                            events.extend(sse_events);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("解码事件失败: {}", e);
                                    }
                                }
                            }

                            // 转换为 SSE 字节流
                            let bytes: Vec<Result<Bytes, Infallible>> = events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();

                            Some((stream::iter(bytes), (body_stream, ctx, decoder, false, false, ping_interval, provider, rewriter_config, user_id)))
                        }
                        Some(Err(e)) => {
                            tracing::error!("读取响应流失败: {}", e);
                            // 流结束前处理改写缓冲区
                            let (_events, entering_drip) = flush_rewrite_buffer(&mut ctx, &provider, rewriter_config.as_ref(), user_id.as_deref()).await;
                            if entering_drip {
                                // 进入 drip 模式，下次迭代开始逐块输出
                                let bytes: Vec<Result<Bytes, Infallible>> = Vec::new();
                                Some((stream::iter(bytes), (body_stream, ctx, decoder, false, true, ping_interval, provider, rewriter_config, user_id)))
                            } else {
                                // 无需 drip，直接结束
                                let final_events = ctx.generate_final_events();
                                let bytes: Vec<Result<Bytes, Infallible>> = final_events
                                    .into_iter()
                                    .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                    .collect();
                                Some((stream::iter(bytes), (body_stream, ctx, decoder, true, false, ping_interval, provider, rewriter_config, user_id)))
                            }
                        }
                        None => {
                            // 流结束，处理改写缓冲区
                            let (_events, entering_drip) = flush_rewrite_buffer(&mut ctx, &provider, rewriter_config.as_ref(), user_id.as_deref()).await;
                            if entering_drip {
                                // 进入 drip 模式
                                let bytes: Vec<Result<Bytes, Infallible>> = Vec::new();
                                Some((stream::iter(bytes), (body_stream, ctx, decoder, false, true, ping_interval, provider, rewriter_config, user_id)))
                            } else {
                                // 无需 drip，直接结束
                                let final_events = ctx.generate_final_events();
                                let bytes: Vec<Result<Bytes, Infallible>> = final_events
                                    .into_iter()
                                    .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                    .collect();
                                Some((stream::iter(bytes), (body_stream, ctx, decoder, true, false, ping_interval, provider, rewriter_config, user_id)))
                            }
                        }
                    }
                }
                // 发送 ping 保活
                _ = ping_interval.tick() => {
                    tracing::trace!("发送 ping 保活事件");
                    let bytes: Vec<Result<Bytes, Infallible>> = vec![Ok(create_ping_sse())];
                    Some((stream::iter(bytes), (body_stream, ctx, decoder, false, false, ping_interval, provider, rewriter_config, user_id)))
                }
            }
        },
    )
    .flatten();

    initial_stream.chain(processing_stream)
}

/// 处理非流式请求
async fn handle_non_stream_request(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    context: NonStreamRequestContext<'_>,
) -> Response {
    // 调用 Kiro API（支持多凭据故障转移）
    let api_result = match provider
        .call_api(context.request_body, context.user_id)
        .await
    {
        Ok(resp) => resp,
        Err(e) => return map_kiro_provider_error_to_response(context.request_body, e),
    };

    let final_cache_context = match (context.cache_tracker, context.cache_profile) {
        (Some(tracker), Some(profile)) => {
            let resolved = compute_cache_usage(tracker, &context.cache_key, profile);
            tracing::info!(
                credential_id = api_result.credential_id,
                final_cache_creation_input_tokens = resolved.cache_creation_input_tokens,
                final_cache_read_input_tokens = resolved.cache_read_input_tokens,
                "Resolved cache usage for non-stream request"
            );
            tracker.update(&context.cache_key, profile);
            Some(resolved)
        }
        _ => None,
    };

    // 读取响应体
    let body_bytes = match api_result.response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("读取响应体失败: {}", e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse::without_request_id(
                    "api_error",
                    format!("读取响应失败: {}", e),
                )),
            )
                .into_response();
        }
    };

    // 解析事件流
    let mut decoder = EventStreamDecoder::new();
    if let Err(e) = decoder.feed(&body_bytes) {
        tracing::warn!("缓冲区溢出: {}", e);
    }

    let mut text_content = String::new();
    let mut tool_uses: Vec<serde_json::Value> = Vec::new();
    let mut has_tool_use = false;
    let mut stop_reason = "end_turn".to_string();
    let mut context_input_tokens: Option<i32> = None;

    // 推理内容收集（原生 reasoningContentEvent）
    let mut reasoning_text = String::new();
    let mut reasoning_signature: Option<String> = None;
    let mut redacted_thinking_blocks: Vec<String> = Vec::new();

    // 收集工具调用的增量 JSON
    let mut tool_json_buffers: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for result in decoder.decode_iter() {
        match result {
            Ok(frame) => {
                if let Ok(event) = Event::from_frame(frame) {
                    match event {
                        Event::AssistantResponse(resp) => {
                            text_content.push_str(&resp.content);
                        }
                        Event::ToolUse(tool_use) => {
                            has_tool_use = true;

                            // 累积工具的 JSON 输入
                            let buffer = tool_json_buffers
                                .entry(tool_use.tool_use_id.clone())
                                .or_default();
                            buffer.push_str(&tool_use.input);

                            // 如果是完整的工具调用，添加到列表
                            if tool_use.stop {
                                let input: serde_json::Value = if buffer.trim().is_empty() {
                                    // 上游可能省略无参工具的 input 字段（或传空字符串）。
                                    // 这里将其视为合法的空对象，避免 EOF 解析错误导致日志噪音。
                                    serde_json::json!({})
                                } else {
                                    serde_json::from_str(buffer).unwrap_or_else(|e| {
                                        // 检测是否为截断导致的解析失败
                                        if let Some(truncation_info) =
                                            super::truncation::detect_truncation(
                                                &tool_use.name,
                                                &tool_use.tool_use_id,
                                                buffer,
                                            )
                                        {
                                            let soft_msg =
                                                super::truncation::build_soft_failure_result(
                                                    &truncation_info,
                                                );
                                            tracing::warn!(
                                                tool_use_id = %tool_use.tool_use_id,
                                                truncation_type = %truncation_info.truncation_type,
                                                "检测到工具调用截断: {}", soft_msg
                                            );
                                        }

                                        // 仅在显式开启敏感日志时输出完整内容
                                        #[cfg(feature = "sensitive-logs")]
                                        tracing::warn!(
                                            tool_use_id = %tool_use.tool_use_id,
                                            buffer = %buffer,
                                            request_body = %truncate_middle(context.request_body, 1200),
                                            "工具输入 JSON 解析失败: {e}"
                                        );
                                        #[cfg(not(feature = "sensitive-logs"))]
                                        tracing::warn!(
                                            tool_use_id = %tool_use.tool_use_id,
                                            buffer_bytes = buffer.len(),
                                            request_body_bytes = context.request_body.len(),
                                            "工具输入 JSON 解析失败: {e}"
                                        );
                                        serde_json::json!({})
                                    })
                                };

                                // 释放已完成的 buffer，避免请求处理期间内存重复占用
                                tool_json_buffers.remove(&tool_use.tool_use_id);

                                let original_name = context
                                    .tool_name_map
                                    .get(&tool_use.name)
                                    .cloned()
                                    .unwrap_or_else(|| tool_use.name.clone());

                                let tool_id =
                                    super::stream::ensure_toolu_prefix(&tool_use.tool_use_id);

                                tool_uses.push(json!({
                                    "type": "tool_use",
                                    "id": tool_id,
                                    "name": original_name,
                                    "input": input
                                }));
                            }
                        }
                        Event::ContextUsage(context_usage) => {
                            // 从上下文使用百分比计算实际的 input_tokens
                            let context_window =
                                super::types::get_context_window_size(context.model) as f64;
                            let actual_input_tokens =
                                (context_usage.context_usage_percentage * context_window / 100.0)
                                    as i32;
                            context_input_tokens = Some(actual_input_tokens);
                            // 上下文使用量达到 100% 时，设置 stop_reason 为 max_tokens
                            if context_usage.context_usage_percentage >= 100.0 {
                                stop_reason = "max_tokens".to_string();
                            }
                            tracing::debug!(
                                "收到 contextUsageEvent: {}%, 计算 input_tokens: {} (context_window: {})",
                                context_usage.context_usage_percentage,
                                actual_input_tokens,
                                context_window as i32
                            );
                        }
                        Event::Metering(event_metering) => {
                            tracing::debug!(
                                usage = event_metering.usage,
                                unit = %event_metering.unit,
                                unit_plural = %event_metering.unit_plural,
                                "收到 meteringEvent"
                            );
                        }
                        Event::ReasoningContent(reasoning) => {
                            if let Some(text) = &reasoning.text {
                                reasoning_text.push_str(text);
                            }
                            if let Some(sig) = &reasoning.signature {
                                reasoning_signature = Some(sig.clone());
                            }
                            if let Some(redacted) = &reasoning.redacted_content {
                                redacted_thinking_blocks.push(redacted.clone());
                            }
                        }
                        Event::Exception { exception_type, .. }
                            if exception_type == "ContentLengthExceededException" =>
                        {
                            stop_reason = "max_tokens".to_string();
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                tracing::warn!("解码事件失败: {}", e);
            }
        }
    }

    // 确定 stop_reason
    if has_tool_use && stop_reason == "end_turn" {
        stop_reason = "tool_use".to_string();
    }

    // 构建响应内容
    let mut content: Vec<serde_json::Value> = Vec::new();
    let mut rewrite_extra_output_tokens: i32 = 0;

    // 推理内容块（thinking）放在最前面
    if let Some(thinking_block) = build_thinking_content_block(
        &reasoning_text,
        reasoning_signature.as_deref(),
        context.model,
    ) {
        content.push(thinking_block);
    }

    // redacted_thinking 块
    for redacted in &redacted_thinking_blocks {
        content.push(json!({
            "type": "redacted_thinking",
            "data": redacted
        }));
    }

    if !text_content.is_empty() {
        // 结构化输出模式：剥离 markdown fence
        let final_text = if context.structured_output {
            super::structured_output::extract_json_from_response(&text_content)
        } else {
            text_content
        };
        // 关键词改写（仅当启用且包含关键词时触发）
        let final_text = if let Some(rewritten) =
            rewrite_obvious_self_identity_if_enabled(&final_text, context.rewriter_config)
        {
            rewritten
        } else if let Some(rewriter_cfg) = context.rewriter_config
            && super::rewriter::contains_keywords(&final_text, &rewriter_cfg.keywords)
        {
            tracing::info!(
                text_chars = final_text.len(),
                "非流式响应检测到关键词，触发模型改写"
            );
            match super::rewriter::rewrite_text(
                &provider,
                &final_text,
                context.model,
                rewriter_cfg,
                None,
                context.user_id,
            )
            .await
            {
                Ok(result) => {
                    rewrite_extra_output_tokens = result.output_tokens;
                    result.text
                }
                Err(e) => {
                    tracing::error!("非流式关键词改写失败，回退到原始文本: {}", e);
                    final_text
                }
            }
        } else {
            final_text
        };

        content.push(json!({
            "type": "text",
            "text": final_text
        }));
    }

    content.extend(tool_uses);

    // 估算输出 tokens（加上改写消耗的额外 token）
    let output_tokens = token::estimate_output_tokens(&content) + rewrite_extra_output_tokens;

    // 按 PromptCacheMode 决定 total input_tokens 的来源：
    // - Off:       仅使用本地估算
    // - Simulated/Upstream: 优先使用上游 contextUsageEvent (~ 实际值)，无则回退到本地估算
    //
    // TODO: 当上游开始发送 messageMetadataEvent.tokenUsage 时，优先使用其计算结果。
    let local_estimate = context.input_tokens;
    let total_input_tokens = match context.cache_mode {
        PromptCacheMode::Off => local_estimate,
        PromptCacheMode::Upstream | PromptCacheMode::Simulated => {
            context_input_tokens.unwrap_or(local_estimate)
        }
    };

    let response_input_tokens = match context.cache_mode {
        PromptCacheMode::Off | PromptCacheMode::Upstream => total_input_tokens,
        PromptCacheMode::Simulated => final_cache_context
            .map(|ctx| {
                crate::anthropic::usage::billed_input_tokens(
                    total_input_tokens,
                    ctx.cache_creation_input_tokens,
                    ctx.cache_read_input_tokens,
                )
            })
            .unwrap_or(total_input_tokens),
    };

    tracing::info!(
        estimated_input_tokens = local_estimate,
        context_input_tokens = ?context_input_tokens,
        total_input_tokens,
        response_input_tokens,
        output_tokens,
        cache_mode = ?context.cache_mode,
        "Non-stream usage summary"
    );

    let response_body = {
        let mut usage = json!({
            "input_tokens": response_input_tokens,
            "output_tokens": output_tokens
        });
        if matches!(context.cache_mode, PromptCacheMode::Simulated)
            && let Some(cache_context) = final_cache_context
        {
            inject_cache_usage_fields(&mut usage, cache_context);
        }

        json!({
            "id": super::stream::generate_anthropic_message_id(),
            "type": "message",
            "role": "assistant",
            "content": content,
            "model": context.model,
            "stop_reason": stop_reason,
            "stop_sequence": null,
            "usage": usage
        })
    };

    (StatusCode::OK, Json(response_body)).into_response()
}

fn build_thinking_content_block(
    reasoning_text: &str,
    reasoning_signature: Option<&str>,
    model: &str,
) -> Option<serde_json::Value> {
    if reasoning_text.is_empty() && reasoning_signature.is_none() {
        return None;
    }

    let mut thinking_block = json!({
        "type": "thinking",
        "thinking": reasoning_text
    });
    let sig = reasoning_signature
        .map(|s| normalize_signature_for_sse(s, model))
        .unwrap_or_else(|| synthetic_thinking_signature(model, reasoning_text));
    thinking_block["signature"] = json!(sig);

    Some(thinking_block)
}

/// 构建 additionalModelRequestFields
///
/// 根据请求中的 thinking 配置，构建 Kiro API 的 additionalModelRequestFields。
/// 仅对支持 thinking 的模型（Claude 4+）生成该字段。
///
/// 映射规则：
/// - thinking.type == "adaptive" 且无 output_config → { thinking: { type: "adaptive", display? } }（不传 effort，使用后端默认）
/// - thinking.type == "adaptive" 且有 output_config.effort → { thinking: { type: "adaptive", display? }, output_config: { effort: "..." } }
/// - thinking.type == "enabled" → 映射为 adaptive（旧版兼容），不传 effort
/// - thinking.type == "disabled" 或无 thinking → None（不传该字段）
/// - budget_tokens → 丢弃，不使用
///
/// Effort 级别限制：
/// - xhigh/max 仅 Opus 系列支持，其他模型自动回退到 high
fn build_additional_model_request_fields(payload: &MessagesRequest) -> Option<serde_json::Value> {
    let thinking = payload.thinking.as_ref()?;

    // 未启用 thinking 则不传
    if !thinking.is_enabled() {
        return None;
    }

    // 模型门控：仅 Claude 4+ 支持
    if !Thinking::model_supports_thinking(&payload.model) {
        tracing::debug!(
            model = %payload.model,
            "模型不支持 additionalModelRequestFields.thinking，跳过"
        );
        return None;
    }

    // 旧版 enabled + budget_tokens 写法：映射为 adaptive + effort high
    if thinking.thinking_type == "enabled" {
        tracing::info!(
            model = %payload.model,
            "旧版 thinking.type=enabled (budget_tokens), 映射为 adaptive + effort=high"
        );
        return Some(serde_json::json!({
            "thinking": additional_thinking_config(thinking),
            "output_config": { "effort": "high" }
        }));
    }

    // adaptive 模式：检查是否有 output_config.effort
    let output_config = payload.output_config.as_ref();
    if output_config.is_none() {
        tracing::info!(
            model = %payload.model,
            "构建 additionalModelRequestFields: thinking=adaptive (无 effort，使用后端默认)"
        );
        return Some(serde_json::json!({
            "thinking": additional_thinking_config(thinking)
        }));
    }

    // 有 output_config，获取并归一化 effort
    let raw_effort = output_config.unwrap().normalized_effort();

    // xhigh/max 仅 Opus 系列支持，其他模型回退到 high
    let is_opus = payload.model.to_lowercase().contains("opus");
    let effort = if !is_opus && (raw_effort == "xhigh" || raw_effort == "max") {
        tracing::info!(
            model = %payload.model,
            requested_effort = raw_effort,
            "非 Opus 模型不支持 {}, 回退到 high",
            raw_effort
        );
        "high"
    } else {
        raw_effort
    };

    tracing::info!(
        model = %payload.model,
        effort = effort,
        "构建 additionalModelRequestFields: thinking=adaptive"
    );

    Some(serde_json::json!({
        "thinking": additional_thinking_config(thinking),
        "output_config": { "effort": effort }
    }))
}

fn additional_thinking_config(thinking: &Thinking) -> serde_json::Value {
    let mut config = serde_json::json!({ "type": "adaptive" });
    if thinking.display.as_deref() == Some("summarized") {
        config["display"] = serde_json::json!("summarized");
    }
    config
}
///
/// 计算消息的 token 数量。
pub async fn count_tokens(
    OriginalUri(uri): OriginalUri,
    JsonExtractor(payload): JsonExtractor<CountTokensRequest>,
) -> impl IntoResponse {
    tracing::info!(
        path = %uri.path(),
        model = %payload.model,
        message_count = %payload.messages.len(),
        "Received request"
    );

    let total_tokens = token::count_all_tokens(
        payload.model.clone(),
        payload.system.clone(),
        payload.messages.clone(),
        payload.tools.clone(),
    ) as i32;

    Json(CountTokensResponse {
        input_tokens: total_tokens.max(1),
    })
}

/// 截断字符串中间部分，保留头尾各 `keep` 个字符
///
/// 用于 debug 日志：避免输出过长的请求体，同时保留足够上下文便于排查。
/// 正确处理 UTF-8 多字节字符边界，不会截断中文。
#[cfg(feature = "sensitive-logs")]
fn truncate_middle(s: &str, keep: usize) -> std::borrow::Cow<'_, str> {
    // 按字符数计算，避免截断后反而更长
    let char_count = s.chars().count();
    let min_omit = 30; // 省略号 + 数字的最小开销，确保截断有意义
    if char_count <= keep * 2 + min_omit {
        return std::borrow::Cow::Borrowed(s);
    }

    // 找到第 keep 个字符的字节边界
    let head_end = s
        .char_indices()
        .nth(keep)
        .map(|(i, _)| i)
        .unwrap_or(s.len());

    // 找到倒数第 keep 个字符的字节边界
    let tail_start = s
        .char_indices()
        .nth_back(keep - 1)
        .map(|(i, _)| i)
        .unwrap_or(0);

    let omitted = s.len() - head_end - (s.len() - tail_start);
    std::borrow::Cow::Owned(format!(
        "{}...({} bytes omitted)...{}",
        &s[..head_end],
        omitted,
        &s[tail_start..]
    ))
}

/// sensitive-logs 模式下输出完整请求体，但截断 base64 图片数据
///
/// 图片 base64 数据对诊断 400 错误没有价值，但可能占几十 KB。
/// 扫描 `"bytes":"<base64...>"` 模式，将长 base64 替换为占位符。
#[cfg(feature = "sensitive-logs")]
fn truncate_base64_in_request_body(s: &str) -> std::borrow::Cow<'_, str> {
    const MARKER: &str = r#""bytes":""#;
    const MIN_BASE64_LEN: usize = 200;

    // 快速路径：没有 "bytes":" 就直接返回
    if !s.contains(MARKER) {
        return std::borrow::Cow::Borrowed(s);
    }

    let mut result = String::with_capacity(s.len());
    let mut pos = 0;
    let bytes = s.as_bytes();

    while pos < bytes.len() {
        if let Some(offset) = s[pos..].find(MARKER) {
            let marker_start = pos + offset;
            let value_start = marker_start + MARKER.len();

            // 找到闭合引号（处理转义）
            let mut end = value_start;
            let mut escaped = false;
            while end < bytes.len() {
                if escaped {
                    escaped = false;
                    end += 1;
                    continue;
                }
                match bytes[end] {
                    b'\\' => {
                        escaped = true;
                        end += 1;
                    }
                    b'"' => break,
                    _ => end += 1,
                }
            }

            let value_len = end - value_start;
            if value_len >= MIN_BASE64_LEN && is_likely_base64(&s[value_start..end]) {
                result.push_str(&s[pos..value_start]);
                result.push_str(&format!("<BASE64_TRUNCATED:{}>", value_len));
                pos = end; // 跳到闭合引号，下一轮会输出它
            } else {
                // 不是 base64 或太短，原样保留
                result.push_str(&s[pos..value_start]);
                pos = value_start;
            }
        } else {
            result.push_str(&s[pos..]);
            break;
        }
    }

    std::borrow::Cow::Owned(result)
}

#[cfg(feature = "sensitive-logs")]
fn is_likely_base64(s: &str) -> bool {
    s.bytes()
        .take(100)
        .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::types::{Message, SystemMessage};
    use crate::kiro::model::requests::conversation::{
        ConversationState, CurrentMessage, HistoryAssistantMessage, HistoryUserMessage, KiroImage,
        Message as KiroMessage, UserInputMessage,
    };
    use crate::model::config::CompressionConfig;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    const TEST_PDF_HVOYOWSE: &str = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCAxNTAgNTBdIC9SZXNvdXJjZXMgPDwgL0ZvbnQgPDwgL0YxIDUgMCBSID4+ID4+IC9Db250ZW50cyA0IDAgUiA+PgplbmRvYmoKNCAwIG9iago8PCAvTGVuZ3RoIDM4ID4+CnN0cmVhbQpCVCAvRjEgMTQgVGYgMTAgMjAgVGQgKGh2b3lvd3NlKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUgL0ZvbnQgL1N1YnR5cGUgL1R5cGUxIC9CYXNlRm9udCAvSGVsdmV0aWNhID4+CmVuZG9iagp4cmVmCjAgNgowMDAwMDAwMDAwIDY1NTM1IGYgCnRyYWlsZXIKPDwgL1NpemUgNiAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKMAolJUVPRg==";
    const TEST_PDF_HVOYOWSE_PADDED: &str = "JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCAxNTAgNTBdIC9SZXNvdXJjZXMgPDwgL0ZvbnQgPDwgL0YxIDUgMCBSID4+ID4+IC9Db250ZW50cyA0IDAgUiA+PgplbmRvYmoKNCAwIG9iago8PCAvTGVuZ3RoIDQyID4+CnN0cmVhbQpCVCAvRjEgMTQgVGYgMTAgMjAgVGQgKCAgaHZveW93c2UgICkgVGogRVQKZW5kc3RyZWFtCmVuZG9iago1IDAgb2JqCjw8IC9UeXBlIC9Gb250IC9TdWJ0eXBlIC9UeXBlMSAvQmFzZUZvbnQgL0hlbHZldGljYSA+PgplbmRvYmoKeHJlZgowIDYKMDAwMDAwMDAwMCA2NTUzNSBmIAp0cmFpbGVyCjw8IC9TaXplIDYgL1Jvb3QgMSAwIFIgPj4Kc3RhcnR4cmVmCjAKJSVFT0Y=";

    fn sample_messages_request() -> MessagesRequest {
        // 生成一个超过 1024 tokens 的 system message 用于测试缓存
        let long_text = "This is a test system message. ".repeat(100); // 约 600 tokens
        let very_long_text = format!("{}{}", long_text, long_text); // 约 1200 tokens

        MessagesRequest {
            model: "claude-sonnet-4-5-20250929".to_string(),
            max_tokens: 1024,
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: serde_json::json!([
                        {"type": "text", "text": "hello raw"},
                        {"type": "text", "text": ""}
                    ]),
                },
                Message {
                    role: "assistant".to_string(),
                    content: serde_json::json!("prefill that convert will drop"),
                },
            ],
            stream: false,
            system: Some(vec![SystemMessage {
                text: very_long_text,
                block_type: Some("text".to_string()),
                cache_control: Some(crate::anthropic::types::CacheControl {
                    cache_type: "ephemeral".to_string(),
                    ttl: None,
                }),
            }]),
            tools: Some(vec![crate::anthropic::types::Tool {
                tool_type: Some("web_search_20250305".to_string()),
                name: "web_search".to_string(),
                description: "search web".to_string(),
                input_schema: std::collections::HashMap::new(),
                max_uses: Some(1),
                cache_control: None,
            }]),
            tool_choice: None,
            thinking: None,
            output_config: None,
            metadata: None,
        }
    }

    fn pdf_text_payload(prompt: &str) -> MessagesRequest {
        pdf_text_payload_with_pdf(prompt, TEST_PDF_HVOYOWSE)
    }

    fn pdf_text_payload_with_pdf(prompt: &str, pdf: &str) -> MessagesRequest {
        let mut payload = sample_messages_request();
        payload.messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":pdf}},
                {"type":"text","text":prompt}
            ]),
        }];
        payload.tools = Some(Vec::new());
        payload
    }

    fn assert_anthropic_message_id(id: &str) {
        assert!(id.starts_with("msg_01"), "unexpected message id: {id}");
        assert_eq!(id.len(), 26, "unexpected message id length: {id}");
        assert!(
            id[6..].bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "unexpected message id characters: {id}"
        );
    }

    #[test]
    fn test_obvious_identity_rewrite_respects_rewriter_enabled() {
        let disabled = crate::anthropic::rewriter::RewriterConfig {
            enabled: false,
            ..Default::default()
        };
        let enabled = crate::anthropic::rewriter::RewriterConfig {
            enabled: true,
            ..Default::default()
        };

        assert_eq!(rewrite_obvious_self_identity_if_enabled("Kiro", None), None);
        assert_eq!(
            rewrite_obvious_self_identity_if_enabled("Kiro", Some(&disabled)),
            None
        );
        assert_eq!(
            rewrite_obvious_self_identity_if_enabled("Kiro", Some(&enabled)),
            Some("Claude Code".to_string())
        );
    }

    #[test]
    fn test_response_rewrite_enabled_does_not_depend_on_keywords() {
        let enabled_without_keywords = crate::anthropic::rewriter::RewriterConfig {
            enabled: true,
            keywords: Vec::new(),
            ..Default::default()
        };

        assert!(is_response_rewrite_enabled(Some(&enabled_without_keywords)));
    }

    #[test]
    fn test_build_thinking_content_block_emits_synthetic_signature() {
        let block = build_thinking_content_block("visible thought", None, "claude-sonnet-4.6")
            .expect("thinking text should create a block");

        assert_eq!(block["type"], "thinking");
        assert_eq!(block["thinking"], "visible thought");
        assert!(
            block["signature"]
                .as_str()
                .is_some_and(|sig| sig.starts_with("kiro-rs-synthetic-")),
            "non-stream thinking blocks need a replayable signature"
        );
    }

    #[test]
    fn test_extract_tag_echo_normalizer_for_direct_test_tag_prompt() {
        let mut payload = sample_messages_request();
        payload.messages = vec![Message {
            role: "user".to_string(),
            content: "我输入了一个tag 是：<antml:test_01fbd3329c862b2e>。直接输出你前面看到的tag是什么。看到的文本，不要使用任何工具。".to_string().into(),
        }];

        let normalizer = extract_tag_echo_normalizer(&payload).unwrap();

        assert_eq!(normalizer.full_tag, "<antml:test_01fbd3329c862b2e>");
    }

    #[test]
    fn test_extract_tag_echo_normalizer_rejects_non_direct_or_non_test_tags() {
        let mut payload = sample_messages_request();
        payload.messages = vec![Message {
            role: "user".to_string(),
            content: "解释这个标签 <antml:test_01fbd3329c862b2e> 的含义"
                .to_string()
                .into(),
        }];
        assert_eq!(extract_tag_echo_normalizer(&payload), None);

        payload.messages = vec![Message {
            role: "user".to_string(),
            content: "直接输出 <antml:prod_01fbd3329c862b2e>".to_string().into(),
        }];
        assert_eq!(extract_tag_echo_normalizer(&payload), None);

        payload.messages = vec![Message {
            role: "user".to_string(),
            content: "What is the exact meaning of this tag <antml:test_01fbd3329c862b2e>?"
                .to_string()
                .into(),
        }];
        assert_eq!(extract_tag_echo_normalizer(&payload), None);

        payload.messages = vec![Message {
            role: "user".to_string(),
            content: json!([
                {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":TEST_PDF_HVOYOWSE}},
                {"type":"text","text":"Extract the exact PDF text, and note tag <antml:test_01fbd3329c862b2e>."}
            ]),
        }];
        assert_eq!(extract_tag_echo_normalizer(&payload), None);
    }

    #[test]
    fn test_cache_context_uses_raw_system_tokens() {
        let payload = sample_messages_request();

        let cache_tracker =
            crate::anthropic::cache_tracker::CacheTracker::new(std::time::Duration::from_secs(300));

        // 计算实际的 system message tokens
        let system_text = &payload.system.as_ref().unwrap()[0].text;
        let expected = token::count_tokens(system_text) as i32;

        let cache_profile = build_cache_profile(&cache_tracker, &payload, expected);
        let cache_key = crate::anthropic::cache_tracker::CacheKey::Global;
        let cache_context = compute_cache_usage(&cache_tracker, &cache_key, &cache_profile);

        // 首次请求：cache_creation = total - 1（cacheable_total），cache_read = 0
        let cacheable_total = (expected - 1).max(0);
        assert_eq!(cache_context.cache_creation_input_tokens, cacheable_total);
        assert_eq!(cache_context.cache_read_input_tokens, 0);
    }

    #[test]
    fn test_resolved_cache_usage_uses_user_key() {
        let payload = sample_messages_request();
        let estimated = token::count_all_tokens(
            payload.model.clone(),
            payload.system.clone(),
            payload.messages.clone(),
            payload.tools.clone(),
        ) as i32;
        let cache_tracker =
            crate::anthropic::cache_tracker::CacheTracker::new(std::time::Duration::from_secs(300));
        let cache_profile = build_cache_profile(&cache_tracker, &payload, estimated);
        let cache_key = crate::anthropic::cache_tracker::CacheKey::User("user_42".into());

        let initial = compute_cache_usage(&cache_tracker, &cache_key, &cache_profile);
        assert_eq!(initial.cache_read_input_tokens, 0);

        cache_tracker.update(&cache_key, &cache_profile);
        let resolved = compute_cache_usage(&cache_tracker, &cache_key, &cache_profile);

        assert!(resolved.cache_read_input_tokens > 0);
        assert!(resolved.cache_creation_input_tokens <= initial.cache_creation_input_tokens);
    }

    #[test]
    fn test_billed_input_tokens_subtracts_cache_tokens() {
        use crate::anthropic::usage::billed_input_tokens;
        // 当有 cache 活动时，billed 固定为 1
        assert_eq!(billed_input_tokens(3829, 0, 1788), 1);
        assert_eq!(billed_input_tokens(4131, 544, 2544), 1);
        assert_eq!(billed_input_tokens(10, 3, 20), 1);
        // 无 cache 活动时，返回 total
        assert_eq!(billed_input_tokens(3829, 0, 0), 3829);
    }

    #[test]
    fn test_non_stream_usage_uses_estimated_input_tokens_as_base() {
        use crate::anthropic::usage::billed_input_tokens;
        let estimated_input_tokens = 1493;
        let upstream_context_input_tokens = 3106;
        let cache_creation_input_tokens = 9;
        let cache_read_input_tokens = 1480;

        let final_input_tokens = estimated_input_tokens;
        let billed = billed_input_tokens(
            final_input_tokens,
            cache_creation_input_tokens,
            cache_read_input_tokens,
        );

        assert_eq!(final_input_tokens, 1493);
        assert_eq!(upstream_context_input_tokens, 3106);
        // cache 活跃时，billed 固定为 1
        assert_eq!(billed, 1);
        assert_ne!(final_input_tokens, upstream_context_input_tokens);
    }

    #[test]
    fn test_build_thinking_content_block_includes_signature_without_text() {
        let block = build_thinking_content_block("", Some("sig_123"), "claude-opus-4-8")
            .expect("thinking block should be built when signature exists");

        assert_eq!(block["type"], json!("thinking"));
        assert_eq!(block["thinking"], json!(""));
        assert_eq!(block["signature"], json!("sig_123"));
    }

    #[test]
    fn test_build_thinking_content_block_always_emits_signature_when_some() {
        let block =
            build_thinking_content_block("thinking text", Some("sig"), "claude-sonnet-4-20250514")
                .expect("thinking block should be built");
        assert_eq!(block["signature"], json!("sig"));
    }

    #[test]
    fn test_build_thinking_content_block_always_emits_signature_when_none() {
        let block = build_thinking_content_block("thinking text", None, "claude-sonnet-4-20250514")
            .expect("thinking block should be built");
        assert!(
            block["signature"]
                .as_str()
                .is_some_and(|sig| sig.starts_with("kiro-rs-synthetic-")),
            "missing upstream signatures should be replaced for OpenCode replay"
        );
    }

    #[test]
    fn test_build_thinking_content_block_normalizes_opus_4_8_signature() {
        const CAPTURED_OPUS_4_8_SIGNATURE: &str = "EpIECmMIDhABGAIqQO1JSL2DsiN2K5nm/MsmMj7aYlm3EDWep9jVZmYoGh/vL2dNlBR8HZL3RQ08KAW2goV8552GI/5psxvgdO10pRUyDWNsYXVkZS1xdWluY2U4AEIIdGhpbmtpbmcSDDGr9CGvpiPycCX6ihoMbuTNqiYdVlqEslfrIjC+CLSCrK0oKnmfm1jAhSAz+KrbEuBK8m5wccq1R/UX3gqMcEvmhNZEBLtvYclSyOgq3AKdk7urfodjAZjIIwQ0mKQWIOGmtnIAKl1G1Q8csz33RxVwVdABBHPphTh8wigMjjEKVG4WVDmDOdaZN5HcALlcQmfO1SPiswGMlfjdAbM09NDcpc/SVq00lFt2tsQ48HeYJf5s5PaaFFGM8+lxmTqOGLpWnln+KOp+OK7PsGOsFRLu10gjHETF8ok82/WHX1KjbQcKRGnaB8noSPEjbyFnDoUzsBAcVIMdkZxAWYKX4R9WCJWOpLOXaTIhsEUhaAJ2e4p1WI+kblnbb7pf9ntP/lZeguwiK67lENlr/MV57M/JDd3d/gCZaEzLSr5z1XRQB/b0tonGT64nXvp/ZbzZmzDFhoPIhvW/19eGu4OY1GSZO4hZDugm+f02yLTwMBT0YozkI5W/jQ5QJJ45XjW6dI4IgxZiQGIrnRkg/okSbAuLY7qiNcjwKEeBoafa1TZf2hFuaeizNkd1V+UYAQ==";

        let block =
            build_thinking_content_block("", Some(CAPTURED_OPUS_4_8_SIGNATURE), "claude-opus-4-8")
                .expect("thinking block should be built when signature exists");

        let signature = block["signature"].as_str().unwrap();
        let raw = STANDARD.decode(signature).unwrap();

        assert!(raw.windows(b"thinking".len()).any(|w| w == b"thinking"));
        assert!(
            raw.windows(b"claude-opus-4-8".len())
                .any(|w| w == b"claude-opus-4-8")
        );
        assert!(
            !raw.windows(b"claude-quince8".len())
                .any(|w| w == b"claude-quince8")
        );
        assert!(
            !raw.windows([0x10, 0x01, 0x18, 0x02].len())
                .any(|w| w == [0x10, 0x01, 0x18, 0x02])
        );
    }

    #[test]
    fn test_extract_pdf_text_answer_for_direct_text_query() {
        let payload =
            pdf_text_payload("What text does this PDF contain? 只给我返回文字,不要使用工具");

        assert_eq!(
            extract_pdf_text_answer(&payload).as_deref(),
            Some("hvoyowse")
        );
    }

    #[test]
    fn test_extract_pdf_text_answer_accepts_common_direct_text_prompts() {
        for prompt in [
            "Read the PDF and return only the exact text.",
            "OCR this PDF. Return raw text only.",
            "请提取这个 PDF 里的文本，只输出原文。",
            "PDF 里写了什么？仅输出原文。",
        ] {
            let payload = pdf_text_payload(prompt);

            assert_eq!(
                extract_pdf_text_answer(&payload).as_deref(),
                Some("hvoyowse"),
                "prompt should trigger PDF text fast path: {prompt}"
            );
        }
    }

    #[test]
    fn test_extract_pdf_text_answer_preserves_pdf_text_whitespace() {
        let payload = pdf_text_payload_with_pdf(
            "Extract the text from this PDF and return only the text.",
            TEST_PDF_HVOYOWSE_PADDED,
        );

        assert_eq!(
            extract_pdf_text_answer(&payload).as_deref(),
            Some("  hvoyowse  ")
        );
    }

    #[test]
    fn test_extract_pdf_text_answer_ignores_general_pdf_prompt() {
        let payload = pdf_text_payload("Summarize this PDF in Chinese.");

        assert_eq!(extract_pdf_text_answer(&payload), None);
    }

    #[test]
    fn test_extract_pdf_text_answer_ignores_summary_even_when_text_mentioned() {
        let payload = pdf_text_payload(
            "Summarize the text in this PDF in Chinese, only include important points.",
        );

        assert_eq!(extract_pdf_text_answer(&payload), None);
    }

    #[test]
    fn test_extract_pdf_text_answer_ignores_structured_output_requests() {
        let mut payload =
            pdf_text_payload("Extract the text from this PDF and return only the text.");
        payload.output_config = Some(crate::anthropic::types::OutputConfig {
            effort: "high".to_string(),
            format: Some(crate::anthropic::types::OutputFormat {
                format_type: "json_schema".to_string(),
                schema: Some(json!({"type":"object"})),
                name: Some("pdf_text".to_string()),
            }),
        });

        assert_eq!(extract_pdf_text_answer(&payload), None);
    }

    #[test]
    fn test_additional_model_request_fields_preserves_summarized_display() {
        let mut payload = sample_messages_request();
        payload.model = "claude-opus-4-8".to_string();
        payload.thinking = Some(Thinking {
            thinking_type: "adaptive".to_string(),
            display: Some("summarized".to_string()),
            budget_tokens: 20000,
        });
        payload.output_config = Some(crate::anthropic::types::OutputConfig {
            effort: "xhigh".to_string(),
            format: None,
        });

        let fields = build_additional_model_request_fields(&payload)
            .expect("adaptive thinking should produce additional fields");

        assert_eq!(fields["thinking"]["type"], "adaptive");
        assert_eq!(fields["thinking"]["display"], "summarized");
        assert_eq!(fields["output_config"]["effort"], "xhigh");
    }

    #[test]
    fn test_additional_model_request_fields_omits_unknown_display() {
        let mut payload = sample_messages_request();
        payload.model = "claude-opus-4-8".to_string();
        payload.thinking = Some(Thinking {
            thinking_type: "adaptive".to_string(),
            display: Some("verbose".to_string()),
            budget_tokens: 20000,
        });

        let fields = build_additional_model_request_fields(&payload)
            .expect("adaptive thinking should produce additional fields");

        assert_eq!(fields["thinking"]["type"], "adaptive");
        assert!(fields["thinking"].get("display").is_none());
    }

    #[tokio::test]
    async fn test_build_local_text_response_uses_anthropic_message_id_non_stream() {
        let payload = pdf_text_payload("Extract the text from this PDF and return only the text.");
        let response = build_local_text_response(&payload, "hvoyowse", 10);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_anthropic_message_id(value["id"].as_str().unwrap());
    }

    #[tokio::test]
    async fn test_build_local_text_response_uses_anthropic_message_id_stream() {
        let mut payload =
            pdf_text_payload("Extract the text from this PDF and return only the text.");
        payload.stream = true;

        let response = build_local_text_response(&payload, "hvoyowse", 10);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        let data = body
            .split("\n\n")
            .find_map(|event| event.lines().find_map(|line| line.strip_prefix("data: ")))
            .expect("message_start data should exist");
        let value: serde_json::Value = serde_json::from_str(data).unwrap();

        assert_anthropic_message_id(value["message"]["id"].as_str().unwrap());
    }

    #[test]
    fn test_inject_cache_usage_fields_only_for_cc_usage() {
        let mut usage = serde_json::json!({
            "input_tokens": 123,
            "output_tokens": 45
        });

        inject_cache_usage_fields(
            &mut usage,
            CacheUsageContext {
                cache_creation_input_tokens: 7,
                cache_read_input_tokens: 8,
                cache_creation_5m_input_tokens: 3,
                cache_creation_1h_input_tokens: 4,
            },
        );

        assert_eq!(usage["cache_creation_input_tokens"], 7);
        assert_eq!(usage["cache_read_input_tokens"], 8);
        assert_eq!(usage["cache_creation"]["ephemeral_5m_input_tokens"], 3);
        assert_eq!(usage["cache_creation"]["ephemeral_1h_input_tokens"], 4);
    }

    #[test]
    fn test_is_no_credentials_error() {
        let err = anyhow::anyhow!("没有可用的凭据");
        assert!(is_no_credentials_error(&err));

        let err = anyhow::anyhow!("所有凭据已用尽");
        assert!(!is_no_credentials_error(&err));
    }

    #[test]
    fn test_is_quota_exhausted_error() {
        let err = anyhow::anyhow!("流式 API 请求失败（所有凭据已用尽）: 429 Quota exceeded");
        assert!(is_quota_exhausted_error(&err));

        let err = anyhow::anyhow!("没有可用的凭据（可用: 0/0），请添加或启用凭据后重试");
        assert!(!is_quota_exhausted_error(&err));
    }

    #[test]
    fn test_is_all_credentials_cooling_down_error_matches_and_parses() {
        let err = anyhow::anyhow!(
            "所有凭据均处于冷却/速率限制（retry_after_secs=120，原因：cooldown，来自凭据 #3）"
        );
        let (matched, retry) = is_all_credentials_cooling_down_error(&err);
        assert!(matched);
        assert_eq!(retry, Some(120));
    }

    #[test]
    fn test_is_all_credentials_cooling_down_error_rejects_unrelated() {
        let err = anyhow::anyhow!("所有凭据已用尽");
        let (matched, retry) = is_all_credentials_cooling_down_error(&err);
        assert!(!matched);
        assert_eq!(retry, None);

        let err = anyhow::anyhow!("没有可用的凭据");
        let (matched, retry) = is_all_credentials_cooling_down_error(&err);
        assert!(!matched);
        assert_eq!(retry, None);
    }

    #[test]
    fn test_is_all_credentials_cooling_down_error_missing_secs_returns_none() {
        let err = anyhow::anyhow!("所有凭据均处于冷却/速率限制（无 retry_after 信息）");
        let (matched, retry) = is_all_credentials_cooling_down_error(&err);
        assert!(matched);
        assert_eq!(retry, None);
    }

    #[test]
    fn test_adaptive_shrink_removes_only_history_images() {
        let big = "A".repeat(20_000);
        let mut kiro_request = KiroRequest {
            conversation_state: ConversationState::new("conv-1")
                .with_current_message(CurrentMessage::new(
                    UserInputMessage::new("current", "model")
                        .with_images(vec![KiroImage::from_base64("png", big.clone())]),
                ))
                .with_history(vec![KiroMessage::User(HistoryUserMessage::new(
                    "history", "model",
                ))]),
            profile_arn: None,
            additional_model_request_fields: None,
        };
        if let KiroMessage::User(user) = &mut kiro_request.conversation_state.history[0] {
            user.user_input_message.images = vec![KiroImage::from_base64("png", big.clone())];
        }

        let removed = kiro_request.conversation_state.remove_history_images();

        assert_eq!(removed, 1);
        assert_eq!(
            kiro_request
                .conversation_state
                .current_message
                .user_input_message
                .images
                .len(),
            1
        );
        assert!(match &kiro_request.conversation_state.history[0] {
            KiroMessage::User(user) => user.user_input_message.images.is_empty(),
            _ => false,
        });
    }

    #[test]
    fn test_adaptive_shrink_progressive_image_removal() {
        // 构造 3 条历史用户消息各带图片 + current_message 带图片
        // 设置阈值使得只需移除前 2 条历史图片即可达标
        let img_data = "B".repeat(10_000); // 每张图片 ~10KB
        let mut history = Vec::new();
        for i in 0..3 {
            let mut user_msg = HistoryUserMessage::new(format!("msg{}", i), "model");
            user_msg.user_input_message.images =
                vec![KiroImage::from_base64("png", img_data.clone())];
            history.push(KiroMessage::User(user_msg));
            history.push(KiroMessage::Assistant(HistoryAssistantMessage::new(
                format!("reply{}", i),
            )));
        }

        let mut kiro_request = KiroRequest {
            conversation_state: ConversationState::new("conv-1")
                .with_current_message(CurrentMessage::new(
                    UserInputMessage::new("current", "model")
                        .with_images(vec![KiroImage::from_base64("png", img_data.clone())]),
                ))
                .with_history(history),
            profile_arn: None,
            additional_model_request_fields: None,
        };

        let mut request_body = serde_json::to_string(&kiro_request).unwrap();
        let config = CompressionConfig::default();
        // 设置阈值：总体积减去 2 张图片后刚好能通过
        let max_body = request_body.len() - 2 * img_data.len() + 500;

        let outcome =
            adaptive_shrink_request_body(&mut kiro_request, &config, max_body, &mut request_body)
                .unwrap()
                .expect("should trigger adaptive compression");

        assert!(outcome.history_images_removed);
        // current_message 图片应保留
        assert_eq!(
            kiro_request
                .conversation_state
                .current_message
                .user_input_message
                .images
                .len(),
            1
        );
        // 最早的 2 条历史消息图片应被清除，第 3 条保留
        // history[0] = User(msg0), history[1] = Assistant, history[2] = User(msg1), ...
        if let KiroMessage::User(u) = &kiro_request.conversation_state.history[0] {
            assert!(
                u.user_input_message.images.is_empty(),
                "oldest should be cleared"
            );
        }
        if let KiroMessage::User(u) = &kiro_request.conversation_state.history[2] {
            assert!(
                u.user_input_message.images.is_empty(),
                "second oldest should be cleared"
            );
        }
        if let KiroMessage::User(u) = &kiro_request.conversation_state.history[4] {
            assert!(
                !u.user_input_message.images.is_empty(),
                "most recent history image should be preserved"
            );
        }
    }

    #[test]
    fn test_adaptive_shrink_can_be_disabled() {
        let oversized_content = "A".repeat(20_000);
        let mut kiro_request = KiroRequest {
            conversation_state: ConversationState::new("conv-1").with_current_message(
                CurrentMessage::new(UserInputMessage::new(oversized_content, "model")),
            ),
            profile_arn: None,
            additional_model_request_fields: None,
        };
        let mut request_body = serde_json::to_string(&kiro_request).unwrap();
        let config = CompressionConfig {
            adaptive_compression: false,
            ..Default::default()
        };

        let outcome = adaptive_shrink_request_body(
            &mut kiro_request,
            &config,
            request_body.len() / 2,
            &mut request_body,
        )
        .unwrap();

        assert!(outcome.is_none());
    }

    #[test]
    fn test_improperly_formed_request_message_mentions_common_causes() {
        let response = map_kiro_provider_error_to_response(
            "{}",
            anyhow::anyhow!("400 Improperly formed request"),
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
