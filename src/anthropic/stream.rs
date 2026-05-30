//! 流式响应处理模块
//!
//! 实现 Kiro → Anthropic 流式响应转换和 SSE 状态管理

use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::json;

use crate::common::utf8::floor_char_boundary;
use crate::kiro::model::events::{Event, MeteringEvent, ReasoningContentEvent};

/// Generate an Anthropic-style message ID: `msg_01` + 20 random alphanumeric characters.
pub fn generate_anthropic_message_id() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let suffix: String = (0..20)
        .map(|_| {
            let idx = fastrand::usize(..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();
    format!("msg_01{}", suffix)
}

/// Ensure a tool_use ID has the `toolu_` prefix expected by the Anthropic API.
pub fn ensure_toolu_prefix(id: &str) -> String {
    if id.starts_with("toolu_") {
        id.to_string()
    } else {
        format!("toolu_{}", id)
    }
}

/// 需要跳过的包裹字符
///
/// 当 thinking 标签被这些字符包裹时，认为是在引用标签而非真正的标签：
/// - 反引号 (`)：行内代码
/// - 双引号 (")：字符串
/// - 单引号 (')：字符串
const QUOTE_CHARS: &[u8] = b"`\"'\\#!@$%^&*()-_=+[]{};:<>,.?/";

/// 检查指定位置的字符是否是引用字符
fn is_quote_char(buffer: &str, pos: usize) -> bool {
    buffer
        .as_bytes()
        .get(pos)
        .map(|c| QUOTE_CHARS.contains(c))
        .unwrap_or(false)
}

/// 查找真正的 thinking 结束标签（不被引用字符包裹，且后面有双换行符）
///
/// 当模型在思考过程中提到 `</thinking>` 时，通常会用反引号、引号等包裹，
/// 或者在同一行有其他内容（如"关于 </thinking> 标签"）。
/// 这个函数会跳过这些情况，只返回真正的结束标签位置。
///
/// 跳过的情况：
/// - 被引用字符包裹（反引号、引号等）
/// - 后面没有双换行符（真正的结束标签后面会有 `\n\n`）
/// - 标签在缓冲区末尾（流式处理时需要等待更多内容）
fn find_real_thinking_end_tag(buffer: &str) -> Option<usize> {
    const TAG: &str = "</thinking>";
    let mut search_start = 0;

    while let Some(pos) = buffer[search_start..].find(TAG) {
        let absolute_pos = search_start + pos;

        // 检查前面是否有引用字符
        let has_quote_before = absolute_pos > 0 && is_quote_char(buffer, absolute_pos - 1);

        // 检查后面是否有引用字符
        let after_pos = absolute_pos + TAG.len();
        let has_quote_after = is_quote_char(buffer, after_pos);

        // 如果被引用字符包裹，跳过
        if has_quote_before || has_quote_after {
            search_start = absolute_pos + 1;
            continue;
        }

        // 检查后面的内容
        let after_content = &buffer[after_pos..];

        // 如果标签后面内容不足以判断是否有双换行符，等待更多内容
        if after_content.len() < 2 {
            return None;
        }

        // 真正的 thinking 结束标签后面会有双换行符 `\n\n`
        if after_content.starts_with("\n\n") {
            return Some(absolute_pos);
        }

        // 不是双换行符，跳过继续搜索
        search_start = absolute_pos + 1;
    }

    None
}

/// 查找缓冲区末尾的 thinking 结束标签（允许末尾只有空白字符）
///
/// 用于"边界事件"场景：例如 thinking 结束后立即进入 tool_use，或流结束，
/// 此时 `</thinking>` 后面可能没有 `\n\n`，但结束标签依然应该识别并过滤。
///
/// 约束：只有当 `</thinking>` 之后全部都是空白字符时才认为是结束标签，
/// 以避免在 thinking 内容中提到 `</thinking>`（非结束标签）时误判。
fn find_real_thinking_end_tag_at_buffer_end(buffer: &str) -> Option<usize> {
    const TAG: &str = "</thinking>";
    let mut search_start = 0;

    while let Some(pos) = buffer[search_start..].find(TAG) {
        let absolute_pos = search_start + pos;

        // 检查前面是否有引用字符
        let has_quote_before = absolute_pos > 0 && is_quote_char(buffer, absolute_pos - 1);

        // 检查后面是否有引用字符
        let after_pos = absolute_pos + TAG.len();
        let has_quote_after = is_quote_char(buffer, after_pos);

        if has_quote_before || has_quote_after {
            search_start = absolute_pos + 1;
            continue;
        }

        // 只有当标签后面全部是空白字符时才认定为结束标签
        if buffer[after_pos..].trim().is_empty() {
            return Some(absolute_pos);
        }

        search_start = absolute_pos + 1;
    }

    None
}

/// 查找真正的 thinking 开始标签（不被引用字符包裹）
/// 与 `find_real_thinking_end_tag` 类似，跳过被引用字符包裹的开始标签。
fn find_real_thinking_start_tag(buffer: &str) -> Option<usize> {
    const TAG: &str = "<thinking>";
    let mut search_start = 0;

    while let Some(pos) = buffer[search_start..].find(TAG) {
        let absolute_pos = search_start + pos;

        // 检查前面是否有引用字符
        let has_quote_before = absolute_pos > 0 && is_quote_char(buffer, absolute_pos - 1);

        // 检查后面是否有引用字符
        let after_pos = absolute_pos + TAG.len();
        let has_quote_after = is_quote_char(buffer, after_pos);

        // 如果不被引用字符包裹，则是真正的开始标签
        if !has_quote_before && !has_quote_after {
            return Some(absolute_pos);
        }

        // 继续搜索下一个匹配
        search_start = absolute_pos + 1;
    }

    None
}

#[cfg(test)]
fn normalize_thinking_signature(sig_b64: &str, allow_any_claude_model: bool) -> Option<String> {
    normalize_thinking_signature_with_mode(sig_b64, allow_any_claude_model, true, None)
}

fn cleanup_existing_thinking_signature(
    sig_b64: &str,
    external_model: Option<&str>,
) -> Option<String> {
    normalize_thinking_signature_with_mode(sig_b64, true, external_model.is_some(), external_model)
}

fn normalize_thinking_signature_with_mode(
    sig_b64: &str,
    allow_any_claude_model: bool,
    insert_thinking_if_missing: bool,
    external_model: Option<&str>,
) -> Option<String> {
    const THINKING_MARKER: &[u8] = b"thinking";
    const NATIVE_MARKER: &[u8] = &[0x10, 0x01, 0x18, 0x02];
    const THINKING_FIELD: &[u8] = b"\x42\x08thinking";

    let raw = BASE64_STANDARD.decode(sig_b64).ok()?;
    if raw.first().copied()? != 0x12 {
        return None;
    }

    let (outer_len, outer_start) = read_varint_after_tag(&raw, 0, 0x12)?;
    let outer_end = outer_start.checked_add(outer_len)?;
    if outer_end > raw.len() || raw.get(outer_start).copied()? != 0x0a {
        return None;
    }

    let (meta_len, meta_start) = read_varint_after_tag(&raw, outer_start, 0x0a)?;
    let meta_end = meta_start.checked_add(meta_len)?;
    if meta_end > outer_end {
        return None;
    }

    let mut meta = raw[meta_start..meta_end].to_vec();
    let model_marker = length_delimited_field_value(&meta, 6)?;
    let model_allowed = if let Some(external_model) = external_model {
        model_marker_matches_external_model(model_marker, external_model)
    } else if allow_any_claude_model {
        is_claude_model_marker(model_marker)
    } else {
        is_4_6_model_marker(model_marker)
    };
    if !model_allowed {
        return None;
    }
    let has_thinking_marker = has_length_delimited_field_value(&meta, 8, THINKING_MARKER);
    if !has_thinking_marker && !insert_thinking_if_missing {
        return None;
    }

    let mut changed = false;
    if let Some((native_start, native_end)) = find_varint_field(&meta, 2, 1) {
        let native_followed_by_reasoning = meta.get(native_end).is_some_and(|_| {
            meta[native_start..]
                .windows(NATIVE_MARKER.len())
                .next()
                .is_some_and(|window| window == NATIVE_MARKER)
        });
        if !native_followed_by_reasoning {
            return None;
        }
        meta.drain(native_start..native_end);
        changed = true;
    } else if !has_thinking_marker {
        return None;
    }

    changed |= normalize_model_alias_marker(&mut meta, external_model);

    if !has_thinking_marker {
        let (_, insert_pos) = find_varint_field(&meta, 7, 0)?;
        meta.splice(insert_pos..insert_pos, THINKING_FIELD.iter().copied());
        changed = true;
    }

    if !changed {
        return Some(sig_b64.to_string());
    }

    let mut rebuilt_outer = Vec::new();
    rebuilt_outer.push(0x0a);
    write_varint(meta.len(), &mut rebuilt_outer);
    rebuilt_outer.extend_from_slice(&meta);
    rebuilt_outer.extend_from_slice(&raw[meta_end..outer_end]);

    let mut rebuilt = Vec::new();
    rebuilt.push(0x12);
    write_varint(rebuilt_outer.len(), &mut rebuilt);
    rebuilt.extend_from_slice(&rebuilt_outer);
    rebuilt.extend_from_slice(&raw[outer_end..]);

    Some(BASE64_STANDARD.encode(rebuilt))
}

pub(super) fn normalize_signature_for_sse(sig: &str, model: &str) -> String {
    let is_thinking_suffix = is_thinking_suffix_model(model);
    let external_model = signature_model_marker_for_request(model);
    if !is_thinking_suffix && !is_4_6_model(model) {
        return cleanup_existing_thinking_signature(sig, external_model.as_deref())
            .unwrap_or_else(|| sig.to_string());
    }
    normalize_thinking_signature_with_mode(sig, is_thinking_suffix, true, external_model.as_deref())
        .unwrap_or_else(|| sig.to_string())
}

fn signature_model_marker_for_request(model: &str) -> Option<String> {
    let lower = model.to_ascii_lowercase();
    let base_model = lower.strip_suffix("-thinking").unwrap_or(&lower);
    let base_model = base_model.strip_suffix("-agentic").unwrap_or(base_model);
    match base_model {
        "claude-opus-4-7" | "claude-opus-4.7" => Some("claude-opus-4-7".to_string()),
        "claude-opus-4-8" | "claude-opus-4.8" => Some("claude-opus-4-8".to_string()),
        _ => None,
    }
}

fn normalize_model_alias_marker(meta: &mut Vec<u8>, external_model: Option<&str>) -> bool {
    let Some(external_model) = external_model else {
        return false;
    };
    let mut changed = false;
    for alias in signature_model_aliases(external_model) {
        while replace_length_delimited_field_value(meta, 6, alias, external_model.as_bytes()) {
            changed = true;
        }
    }
    changed
}

fn model_marker_matches_external_model(marker: &[u8], external_model: &str) -> bool {
    marker == external_model.as_bytes() || signature_model_aliases(external_model).contains(&marker)
}

fn signature_model_aliases(external_model: &str) -> &'static [&'static [u8]] {
    match external_model {
        "claude-opus-4-7" => &[b"claude-quince7", b"claude-quince", b"claude-opus-4.7"],
        "claude-opus-4-8" => &[b"claude-quince8", b"claude-quince", b"claude-opus-4.8"],
        _ => &[],
    }
}

fn is_thinking_suffix_model(model: &str) -> bool {
    model.to_ascii_lowercase().ends_with("-thinking")
}

fn is_4_6_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("4-6") || model.contains("4.6")
}

fn is_4_6_model_marker(marker: &[u8]) -> bool {
    std::str::from_utf8(marker)
        .map(is_4_6_model)
        .unwrap_or(false)
}

fn is_claude_model_marker(marker: &[u8]) -> bool {
    std::str::from_utf8(marker)
        .map(|model| model.to_ascii_lowercase().starts_with("claude-"))
        .unwrap_or(false)
}

fn read_varint_after_tag(data: &[u8], tag_pos: usize, expected_tag: u8) -> Option<(usize, usize)> {
    if data.get(tag_pos).copied()? != expected_tag {
        return None;
    }
    read_varint(data, tag_pos + 1)
}

fn read_varint(data: &[u8], mut pos: usize) -> Option<(usize, usize)> {
    let mut value = 0usize;
    let mut shift = 0usize;

    loop {
        let byte = *data.get(pos)?;
        value |= ((byte & 0x7f) as usize) << shift;
        pos += 1;
        if byte & 0x80 == 0 {
            return Some((value, pos));
        }
        shift += 7;
        if shift >= usize::BITS as usize {
            return None;
        }
    }
}

fn write_varint(mut value: usize, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn find_varint_field(
    data: &[u8],
    target_field: usize,
    target_value: usize,
) -> Option<(usize, usize)> {
    let mut pos = 0;
    while pos < data.len() {
        let field_start = pos;
        let (key, value_start) = read_varint(data, pos)?;
        let field_number = key >> 3;
        let wire_type = key & 0x07;
        pos = value_start;

        match wire_type {
            0 => {
                let (value, field_end) = read_varint(data, pos)?;
                if field_number == target_field && value == target_value {
                    return Some((field_start, field_end));
                }
                pos = field_end;
            }
            1 => pos = pos.checked_add(8)?,
            2 => {
                let (len, payload_start) = read_varint(data, pos)?;
                pos = payload_start.checked_add(len)?;
            }
            5 => pos = pos.checked_add(4)?,
            _ => return None,
        }
    }
    None
}

fn length_delimited_field_value(data: &[u8], target_field: usize) -> Option<&[u8]> {
    let mut pos = 0;
    while pos < data.len() {
        let (key, value_start) = read_varint(data, pos)?;
        let field_number = key >> 3;
        let wire_type = key & 0x07;
        pos = value_start;

        match wire_type {
            0 => {
                let (_, field_end) = read_varint(data, pos)?;
                pos = field_end;
            }
            1 => pos = pos.checked_add(8)?,
            2 => {
                let (len, payload_start) = read_varint(data, pos)?;
                let payload_end = payload_start.checked_add(len)?;
                if payload_end > data.len() {
                    return None;
                }
                if field_number == target_field {
                    return Some(&data[payload_start..payload_end]);
                }
                pos = payload_end;
            }
            5 => pos = pos.checked_add(4)?,
            _ => return None,
        }
    }
    None
}

fn replace_length_delimited_field_value(
    data: &mut Vec<u8>,
    target_field: usize,
    from: &[u8],
    to: &[u8],
) -> bool {
    let mut pos = 0;
    while pos < data.len() {
        let Some((key, value_start)) = read_varint(data, pos) else {
            return false;
        };
        let field_number = key >> 3;
        let wire_type = key & 0x07;
        pos = value_start;

        match wire_type {
            0 => {
                let Some((_, field_end)) = read_varint(data, pos) else {
                    return false;
                };
                pos = field_end;
            }
            1 => {
                let Some(next_pos) = pos.checked_add(8) else {
                    return false;
                };
                pos = next_pos;
            }
            2 => {
                let len_start = pos;
                let Some((len, payload_start)) = read_varint(data, pos) else {
                    return false;
                };
                let Some(payload_end) = payload_start.checked_add(len) else {
                    return false;
                };
                if payload_end > data.len() {
                    return false;
                }
                if field_number == target_field
                    && data.get(payload_start..payload_end) == Some(from)
                {
                    let mut replacement = Vec::new();
                    write_varint(to.len(), &mut replacement);
                    replacement.extend_from_slice(to);
                    data.splice(len_start..payload_end, replacement);
                    return true;
                }
                pos = payload_end;
            }
            5 => {
                let Some(next_pos) = pos.checked_add(4) else {
                    return false;
                };
                pos = next_pos;
            }
            _ => return false,
        }
    }

    false
}

fn has_length_delimited_field_value(data: &[u8], target_field: usize, target_value: &[u8]) -> bool {
    length_delimited_field_value(data, target_field).is_some_and(|value| value == target_value)
}

/// SSE 事件
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: String,
    pub data: serde_json::Value,
}

impl SseEvent {
    pub fn new(event: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }

    /// 格式化为 SSE 字符串
    pub fn to_sse_string(&self) -> String {
        format!(
            "event: {}\ndata: {}\n\n",
            self.event,
            serde_json::to_string(&self.data).unwrap_or_default()
        )
    }
}

/// 内容块状态
#[derive(Debug, Clone)]
struct BlockState {
    block_type: String,
    started: bool,
    stopped: bool,
}

impl BlockState {
    fn new(block_type: impl Into<String>) -> Self {
        Self {
            block_type: block_type.into(),
            started: false,
            stopped: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FinalUsage {
    output_tokens: i32,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct CacheUsageBreakdown {
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
    pub cache_creation_5m_input_tokens: i32,
    pub cache_creation_1h_input_tokens: i32,
}

/// SSE 状态管理器
///
/// 确保 SSE 事件序列符合 Claude API 规范：
/// 1. message_start 只能出现一次
/// 2. content_block 必须先 start 再 delta 再 stop
/// 3. message_delta 只能出现一次，且在所有 content_block_stop 之后
/// 4. message_stop 在最后
#[derive(Debug)]
pub struct SseStateManager {
    /// message_start 是否已发送
    message_started: bool,
    /// message_delta 是否已发送
    message_delta_sent: bool,
    /// 活跃的内容块状态
    active_blocks: HashMap<i32, BlockState>,
    /// 消息是否已结束
    message_ended: bool,
    /// 下一个块索引
    next_block_index: i32,
    /// 当前 stop_reason
    stop_reason: Option<String>,
    /// 是否有工具调用
    has_tool_use: bool,
}

impl Default for SseStateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SseStateManager {
    pub fn new() -> Self {
        Self {
            message_started: false,
            message_delta_sent: false,
            active_blocks: HashMap::new(),
            message_ended: false,
            next_block_index: 0,
            stop_reason: None,
            has_tool_use: false,
        }
    }

    /// 判断指定块是否处于可接收 delta 的打开状态
    fn is_block_open_of_type(&self, index: i32, expected_type: &str) -> bool {
        self.active_blocks
            .get(&index)
            .is_some_and(|b| b.started && !b.stopped && b.block_type == expected_type)
    }

    /// 获取下一个块索引
    pub fn next_block_index(&mut self) -> i32 {
        let index = self.next_block_index;
        self.next_block_index += 1;
        index
    }

    /// 记录工具调用
    pub fn set_has_tool_use(&mut self, has: bool) {
        self.has_tool_use = has;
    }

    /// stop_reason 优先级（索引越小优先级越高）
    const STOP_REASON_PRIORITY: &'static [&'static str] = &[
        "max_tokens",
        "refusal",
        "pause_turn",
        "tool_use",
        "end_turn",
    ];

    /// 获取 stop_reason 的优先级（越小越高，未知原因返回 usize::MAX）
    fn stop_reason_priority(reason: &str) -> usize {
        Self::STOP_REASON_PRIORITY
            .iter()
            .position(|&r| r == reason)
            .unwrap_or(usize::MAX)
    }

    /// 设置 stop_reason（高优先级原因可覆盖低优先级原因）
    ///
    /// 优先级从高到低：max_tokens > refusal > pause_turn > tool_use > end_turn
    pub fn set_stop_reason(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        let new_priority = Self::stop_reason_priority(&reason);
        let should_set = match &self.stop_reason {
            None => true,
            Some(current) => new_priority < Self::stop_reason_priority(current),
        };
        if should_set {
            self.stop_reason = Some(reason);
        }
    }

    /// 检查是否存在非 thinking 类型的内容块（如 text 或 tool_use）
    fn has_non_thinking_blocks(&self) -> bool {
        self.active_blocks
            .values()
            .any(|b| b.block_type != "thinking" && b.block_type != "redacted_thinking")
    }

    /// 获取最终的 stop_reason
    pub fn get_stop_reason(&self) -> String {
        if let Some(ref reason) = self.stop_reason {
            reason.clone()
        } else if self.has_tool_use {
            "tool_use".to_string()
        } else {
            "end_turn".to_string()
        }
    }

    /// 处理 message_start 事件
    pub fn handle_message_start(&mut self, event: serde_json::Value) -> Option<SseEvent> {
        if self.message_started {
            tracing::debug!("跳过重复的 message_start 事件");
            return None;
        }
        self.message_started = true;
        Some(SseEvent::new("message_start", event))
    }

    /// 处理 content_block_start 事件
    pub fn handle_content_block_start(
        &mut self,
        index: i32,
        block_type: &str,
        data: serde_json::Value,
    ) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // 如果是 tool_use 块，先关闭之前的文本块
        if block_type == "tool_use" {
            self.has_tool_use = true;
            for (block_index, block) in self.active_blocks.iter_mut() {
                if block.block_type == "text" && block.started && !block.stopped {
                    // 自动发送 content_block_stop 关闭文本块
                    events.push(SseEvent::new(
                        "content_block_stop",
                        json!({
                            "type": "content_block_stop",
                            "index": block_index
                        }),
                    ));
                    block.stopped = true;
                }
            }
        }

        // 检查块是否已存在
        if let Some(block) = self.active_blocks.get_mut(&index) {
            if block.started {
                tracing::trace!("块 {} 已启动，跳过重复的 content_block_start", index);
                return events;
            }
            block.started = true;
        } else {
            let mut block = BlockState::new(block_type);
            block.started = true;
            self.active_blocks.insert(index, block);
        }

        events.push(SseEvent::new("content_block_start", data));
        events
    }

    /// 处理 content_block_delta 事件
    pub fn handle_content_block_delta(
        &mut self,
        index: i32,
        data: serde_json::Value,
    ) -> Option<SseEvent> {
        // 确保块已启动
        if let Some(block) = self.active_blocks.get(&index) {
            if !block.started || block.stopped {
                tracing::warn!(
                    "块 {} 状态异常: started={}, stopped={}",
                    index,
                    block.started,
                    block.stopped
                );
                return None;
            }
        } else {
            // 块不存在，可能需要先创建
            tracing::warn!("收到未知块 {} 的 delta 事件", index);
            return None;
        }

        Some(SseEvent::new("content_block_delta", data))
    }

    /// 处理 content_block_stop 事件
    pub fn handle_content_block_stop(&mut self, index: i32) -> Option<SseEvent> {
        if let Some(block) = self.active_blocks.get_mut(&index) {
            if block.stopped {
                tracing::debug!("块 {} 已停止，跳过重复的 content_block_stop", index);
                return None;
            }
            block.stopped = true;
            return Some(SseEvent::new(
                "content_block_stop",
                json!({
                    "type": "content_block_stop",
                    "index": index
                }),
            ));
        }
        None
    }

    /// 生成最终事件序列
    pub fn generate_final_events(&mut self, usage: FinalUsage) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // 关闭所有未关闭的块
        for (index, block) in self.active_blocks.iter_mut() {
            if block.started && !block.stopped {
                events.push(SseEvent::new(
                    "content_block_stop",
                    json!({
                        "type": "content_block_stop",
                        "index": index
                    }),
                ));
                block.stopped = true;
            }
        }

        // 发送 message_delta
        if !self.message_delta_sent {
            self.message_delta_sent = true;
            let usage_json = json!({
                "output_tokens": usage.output_tokens,
            });
            events.push(SseEvent::new(
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": self.get_stop_reason(),
                        "stop_sequence": null
                    },
                    "usage": usage_json
                }),
            ));
        }

        // 发送 message_stop
        if !self.message_ended {
            self.message_ended = true;
            events.push(SseEvent::new(
                "message_stop",
                json!({ "type": "message_stop" }),
            ));
        }

        events
    }
}

/// 流处理上下文
pub struct StreamContext {
    /// SSE 状态管理器
    pub state_manager: SseStateManager,
    /// 请求的模型名称
    pub model: String,
    /// 消息 ID
    pub message_id: String,
    /// 输入 tokens（估算值）
    pub input_tokens: i32,
    /// cache usage 统计（可选）
    pub cache_usage: Option<CacheUsageBreakdown>,
    /// 从 contextUsageEvent 计算的实际输入 tokens
    pub context_input_tokens: Option<i32>,
    /// 输出 tokens 累计
    pub output_tokens: i32,
    /// 工具块索引映射 (tool_id -> block_index)
    pub tool_block_indices: HashMap<String, i32>,
    /// 工具名称反向映射（短名称 → 原始名称），用于响应时还原
    pub tool_name_map: HashMap<String, String>,
    /// thinking 是否启用
    pub thinking_enabled: bool,
    /// thinking 内容缓冲区
    pub thinking_buffer: String,
    /// 是否在 thinking 块内
    pub in_thinking_block: bool,
    /// thinking 块是否已提取完成
    pub thinking_extracted: bool,
    /// thinking 块索引
    pub thinking_block_index: Option<i32>,
    /// 待发送的 signature（收到 signature 后暂存，关闭 thinking 块时发送）
    pub pending_signature: Option<String>,
    /// 文本块索引（按需动态分配）
    pub text_block_index: Option<i32>,
    /// 上游 meteringEvent 透传的 credit usage，仅用于最终 usage 统计，不生成独立 SSE 事件
    pub metering: Option<MeteringEvent>,
    /// 是否需要剥离 thinking 内容开头的换行符
    /// 模型输出 `<thinking>\n` 时，`\n` 可能与标签在同一 chunk 或下一 chunk
    strip_thinking_leading_newline: bool,
    /// 是否启用结构化输出（缓冲所有文本，流结束时统一剥离 fence）
    pub structured_output: bool,
    /// 结构化输出缓冲区
    pub structured_output_buffer: String,
    /// 是否启用关键词改写（缓冲 text block 文本，block 结束时检测并改写）
    pub rewrite_enabled: bool,
    /// 改写关键词列表
    pub rewrite_keywords: Vec<String>,
    /// 改写文本缓冲区（缓冲当前 text block 的所有文本）
    pub rewrite_text_buffer: String,
    /// 改写消耗的额外 token（input + output，合并到最终 usage）
    pub rewrite_extra_output_tokens: i32,
    /// 改写后待逐块输出的文本队列（模拟逐字流式输出）
    pub rewrite_drip_queue: std::collections::VecDeque<String>,
}

impl StreamContext {
    /// 创建启用thinking的StreamContext
    pub fn new_with_thinking(
        model: impl Into<String>,
        input_tokens: i32,
        cache_usage: Option<CacheUsageBreakdown>,
        thinking_enabled: bool,
        tool_name_map: HashMap<String, String>,
        structured_output: bool,
        rewrite_keywords: Vec<String>,
    ) -> Self {
        let rewrite_enabled = !rewrite_keywords.is_empty();
        Self {
            state_manager: SseStateManager::new(),
            model: model.into(),
            message_id: generate_anthropic_message_id(),
            input_tokens,
            cache_usage,
            context_input_tokens: None,
            output_tokens: 0,
            tool_block_indices: HashMap::new(),
            tool_name_map,
            thinking_enabled,
            thinking_buffer: String::new(),
            in_thinking_block: false,
            thinking_extracted: false,
            thinking_block_index: None,
            pending_signature: None,
            text_block_index: None,
            metering: None,
            strip_thinking_leading_newline: false,
            structured_output,
            structured_output_buffer: String::new(),
            rewrite_enabled,
            rewrite_keywords,
            rewrite_text_buffer: String::new(),
            rewrite_extra_output_tokens: 0,
            rewrite_drip_queue: std::collections::VecDeque::new(),
        }
    }

    /// 生成 message_start 事件
    pub fn create_message_start_event(&self) -> serde_json::Value {
        let billed_input_tokens = self
            .cache_usage
            .map(|cache_usage| {
                billed_input_tokens(
                    self.input_tokens,
                    cache_usage.cache_creation_input_tokens,
                    cache_usage.cache_read_input_tokens,
                )
            })
            .unwrap_or(self.input_tokens);
        let mut usage = json!({
            "input_tokens": billed_input_tokens,
            "output_tokens": 0,
        });
        if let Some(cache_usage) = self.cache_usage {
            usage["cache_creation_input_tokens"] = json!(cache_usage.cache_creation_input_tokens);
            usage["cache_read_input_tokens"] = json!(cache_usage.cache_read_input_tokens);
        }
        json!({
            "type": "message_start",
            "message": {
                "id": self.message_id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": self.model,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": usage
            }
        })
    }

    /// 生成初始事件序列（仅 message_start）
    ///
    /// 注意：不再在初始化阶段创建空 text block。
    /// 否则当模型首个输出为 tool_use（且没有任何 text_delta）时，
    /// 会产生一个空的 text content block（text=""），客户端写回 history 后会触发上游校验拒绝。
    pub fn generate_initial_events(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // message_start
        let msg_start = self.create_message_start_event();
        if let Some(event) = self.state_manager.handle_message_start(msg_start) {
            events.push(event);
        }

        events
    }

    /// 处理 Kiro 事件并转换为 Anthropic SSE 事件
    pub fn process_kiro_event(&mut self, event: &Event) -> Vec<SseEvent> {
        match event {
            Event::AssistantResponse(resp) => self.process_assistant_response(&resp.content),
            Event::ToolUse(tool_use) => self.process_tool_use(tool_use),
            Event::ReasoningContent(reasoning) => self.process_reasoning_content(reasoning),
            Event::ContextUsage(context_usage) => {
                // 从上下文使用百分比计算实际的 input_tokens
                let context_window = super::types::get_context_window_size(&self.model) as f64;
                let actual_input_tokens =
                    (context_usage.context_usage_percentage * context_window / 100.0) as i32;
                self.context_input_tokens = Some(actual_input_tokens);
                // 上下文使用量达到 100% 时，设置 stop_reason 为 max_tokens
                if context_usage.context_usage_percentage >= 100.0 {
                    self.state_manager.set_stop_reason("max_tokens");
                }
                tracing::debug!(
                    "收到 contextUsageEvent: {:.4}%, 计算 input_tokens: {} (context_window: {})",
                    context_usage.context_usage_percentage,
                    actual_input_tokens,
                    context_window as i32
                );
                Vec::new()
            }
            Event::Metering(metering) => {
                self.metering = Some(metering.clone());
                tracing::debug!(
                    usage = metering.usage,
                    unit = %metering.unit,
                    unit_plural = %metering.unit_plural,
                    "收到 meteringEvent"
                );
                Vec::new()
            }
            Event::Error {
                error_code,
                error_message,
            } => {
                tracing::error!("收到错误事件: {} - {}", error_code, error_message);
                Vec::new()
            }
            Event::Exception {
                exception_type,
                message,
            } => {
                // 处理 ContentLengthExceededException
                if exception_type == "ContentLengthExceededException" {
                    self.state_manager.set_stop_reason("max_tokens");
                }
                tracing::warn!("收到异常事件: {} - {}", exception_type, message);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// 处理助手响应事件
    fn process_assistant_response(&mut self, content: &str) -> Vec<SseEvent> {
        if content.is_empty() {
            return Vec::new();
        }

        // 估算 tokens
        self.output_tokens += estimate_tokens(content);

        // 结构化输出模式：缓冲文本，流结束时统一处理
        if self.structured_output {
            self.structured_output_buffer.push_str(content);
            return Vec::new();
        }

        // 改写模式：缓冲 text block 文本，延迟到 block 结束时统一检测/改写
        if self.rewrite_enabled {
            self.rewrite_text_buffer.push_str(content);
            return Vec::new();
        }

        // 如果 thinking 启用，解析 <thinking> 标签（作为 reasoningContentEvent 的回退路径）
        if self.thinking_enabled {
            return self.process_content_with_thinking(content);
        }

        // 统一使用 text_delta 发送逻辑，
        // 在 tool_use 自动关闭文本块后能够自愈重建新的文本块，避免"吞字"。
        self.create_text_delta_events(content)
    }

    /// 处理包含thinking块的内容（<thinking> 标签解析状态机）
    fn process_content_with_thinking(&mut self, content: &str) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // 将内容添加到缓冲区进行处理
        self.thinking_buffer.push_str(content);

        loop {
            if !self.in_thinking_block && !self.thinking_extracted {
                // 查找 <thinking> 开始标签（跳过被反引号包裹的）
                if let Some(start_pos) = find_real_thinking_start_tag(&self.thinking_buffer) {
                    // 发送 <thinking> 之前的内容作为 text_delta
                    // 注意：如果前面只是空白字符（如 adaptive 模式返回的 \n\n），则跳过，
                    // 避免在 thinking 块之前产生无意义的 text 块导致客户端解析失败
                    let before_thinking = self.thinking_buffer[..start_pos].to_string();
                    if !before_thinking.is_empty() && !before_thinking.trim().is_empty() {
                        events.extend(self.create_text_delta_events(&before_thinking));
                    }

                    // 进入 thinking 块
                    self.in_thinking_block = true;
                    self.strip_thinking_leading_newline = true;
                    self.thinking_buffer =
                        self.thinking_buffer[start_pos + "<thinking>".len()..].to_string();

                    // 创建 thinking 块的 content_block_start 事件
                    let thinking_index = self.state_manager.next_block_index();
                    self.thinking_block_index = Some(thinking_index);
                    let start_events = self.state_manager.handle_content_block_start(
                        thinking_index,
                        "thinking",
                        json!({
                            "type": "content_block_start",
                            "index": thinking_index,
                            "content_block": {
                                "type": "thinking",
                                "thinking": ""
                            }
                        }),
                    );
                    events.extend(start_events);
                } else {
                    // 没有找到 <thinking>，检查是否可能是部分标签
                    // 保留可能是部分标签的内容
                    let target_len = self
                        .thinking_buffer
                        .len()
                        .saturating_sub("<thinking>".len());
                    let safe_len = floor_char_boundary(&self.thinking_buffer, target_len);
                    if safe_len > 0 {
                        let safe_content = self.thinking_buffer[..safe_len].to_string();
                        // 如果 thinking 尚未提取，且安全内容只是空白字符，
                        // 则不发送为 text_delta，继续保留在缓冲区等待更多内容。
                        // 这避免了 4.6 模型中 <thinking> 标签跨事件分割时，
                        // 前导空白（如 "\n\n"）被错误地创建为 text 块，
                        // 导致 text 块先于 thinking 块出现的问题。
                        if !safe_content.is_empty() && !safe_content.trim().is_empty() {
                            events.extend(self.create_text_delta_events(&safe_content));
                            self.thinking_buffer = self.thinking_buffer[safe_len..].to_string();
                        }
                    }
                    break;
                }
            } else if self.in_thinking_block {
                // 剥离 <thinking> 标签后紧跟的换行符（可能跨 chunk）
                if self.strip_thinking_leading_newline {
                    if self.thinking_buffer.starts_with('\n') {
                        self.thinking_buffer = self.thinking_buffer[1..].to_string();
                        self.strip_thinking_leading_newline = false;
                    } else if !self.thinking_buffer.is_empty() {
                        // buffer 非空但不以 \n 开头，不再需要剥离
                        self.strip_thinking_leading_newline = false;
                    }
                    // buffer 为空时保留标志，等待下一个 chunk
                }

                // 在 thinking 块内，查找 </thinking> 结束标签（跳过被反引号包裹的）
                if let Some(end_pos) = find_real_thinking_end_tag(&self.thinking_buffer) {
                    // 提取 thinking 内容
                    let thinking_content = self.thinking_buffer[..end_pos].to_string();
                    if let Some(thinking_index) = self.thinking_block_index
                        && !thinking_content.is_empty()
                    {
                        events.push(
                            self.create_thinking_delta_event(thinking_index, &thinking_content),
                        );
                    }

                    // 结束 thinking 块
                    self.in_thinking_block = false;
                    self.thinking_extracted = true;

                    // 发送空的 thinking_delta 事件，然后发送 content_block_stop 事件
                    if let Some(thinking_index) = self.thinking_block_index {
                        // 先发送空的 thinking_delta
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                        // 再发送 content_block_stop
                        if let Some(stop_event) =
                            self.state_manager.handle_content_block_stop(thinking_index)
                        {
                            events.push(stop_event);
                        }
                    }

                    // 剥离 `</thinking>\n\n`（find_real_thinking_end_tag 已确认 \n\n 存在）
                    self.thinking_buffer =
                        self.thinking_buffer[end_pos + "</thinking>\n\n".len()..].to_string();
                } else {
                    // 没有找到结束标签，发送当前缓冲区内容作为 thinking_delta。
                    // 保留末尾可能是部分 `</thinking>\n\n` 的内容：
                    // find_real_thinking_end_tag 要求标签后有 `\n\n` 才返回 Some，
                    // 因此保留区必须覆盖 `</thinking>\n\n` 的完整长度（13 字节），
                    // 否则当 `</thinking>` 已在 buffer 但 `\n\n` 尚未到达时，
                    // 标签的前几个字符会被错误地作为 thinking_delta 发出。
                    let target_len = self
                        .thinking_buffer
                        .len()
                        .saturating_sub("</thinking>\n\n".len());
                    let safe_len = floor_char_boundary(&self.thinking_buffer, target_len);
                    if safe_len > 0 {
                        let safe_content = self.thinking_buffer[..safe_len].to_string();
                        if let Some(thinking_index) = self.thinking_block_index
                            && !safe_content.is_empty()
                        {
                            events.push(
                                self.create_thinking_delta_event(thinking_index, &safe_content),
                            );
                        }
                        self.thinking_buffer = self.thinking_buffer[safe_len..].to_string();
                    }
                    break;
                }
            } else {
                // thinking 已提取完成，剩余内容作为 text_delta
                if !self.thinking_buffer.is_empty() {
                    let remaining = self.thinking_buffer.clone();
                    self.thinking_buffer.clear();
                    events.extend(self.create_text_delta_events(&remaining));
                }
                break;
            }
        }

        events
    }

    /// 创建 text_delta 事件
    ///
    /// 如果文本块尚未创建，会先创建文本块。
    /// 当发生 tool_use 时，状态机会自动关闭当前文本块；后续文本会自动创建新的文本块继续输出。
    ///
    /// 返回值包含可能的 content_block_start 事件和 content_block_delta 事件。
    fn create_text_delta_events(&mut self, text: &str) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // 如果当前 text_block_index 指向的块已经被关闭（例如 tool_use 开始时自动 stop），
        // 则丢弃该索引并创建新的文本块继续输出，避免 delta 被状态机拒绝导致"吞字"。
        if let Some(idx) = self.text_block_index
            && !self.state_manager.is_block_open_of_type(idx, "text")
        {
            self.text_block_index = None;
        }

        // 获取或创建文本块索引
        let text_index = if let Some(idx) = self.text_block_index {
            idx
        } else {
            // 文本块尚未创建，需要先创建
            let idx = self.state_manager.next_block_index();
            self.text_block_index = Some(idx);

            // 发送 content_block_start 事件
            let start_events = self.state_manager.handle_content_block_start(
                idx,
                "text",
                json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": {
                        "type": "text",
                        "text": ""
                    }
                }),
            );
            events.extend(start_events);
            idx
        };

        // 发送 content_block_delta 事件
        if let Some(delta_event) = self.state_manager.handle_content_block_delta(
            text_index,
            json!({
                "type": "content_block_delta",
                "index": text_index,
                "delta": {
                    "type": "text_delta",
                    "text": text
                }
            }),
        ) {
            events.push(delta_event);
        }

        events
    }

    /// 创建 thinking_delta 事件
    fn create_thinking_delta_event(&self, index: i32, thinking: &str) -> SseEvent {
        SseEvent::new(
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": index,
                "delta": {
                    "type": "thinking_delta",
                    "thinking": thinking
                }
            }),
        )
    }

    /// 处理原生推理内容事件（reasoningContentEvent）
    fn process_reasoning_content(&mut self, reasoning: &ReasoningContentEvent) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // 处理 redacted_content：作为独立的 redacted_thinking 块
        if let Some(redacted) = &reasoning.redacted_content {
            // 如果当前有打开的 thinking 块，先关闭它
            if self.in_thinking_block {
                if let Some(thinking_index) = self.thinking_block_index {
                    // 发送 signature_delta（如果有待发送的签名）
                    if let Some(sig) = self.pending_signature.take() {
                        let normalized_sig = normalize_signature_for_sse(&sig, &self.model);
                        events.push(SseEvent::new(
                            "content_block_delta",
                            json!({
                                "type": "content_block_delta",
                                "index": thinking_index,
                                "delta": {
                                    "type": "signature_delta",
                                    "signature": normalized_sig
                                }
                            }),
                        ));
                    }
                    if let Some(stop_event) =
                        self.state_manager.handle_content_block_stop(thinking_index)
                    {
                        events.push(stop_event);
                    }
                }
                self.in_thinking_block = false;
            }

            // 发送 redacted_thinking content block
            let redacted_index = self.state_manager.next_block_index();
            let start_events = self.state_manager.handle_content_block_start(
                redacted_index,
                "redacted_thinking",
                json!({
                    "type": "content_block_start",
                    "index": redacted_index,
                    "content_block": {
                        "type": "redacted_thinking",
                        "data": redacted
                    }
                }),
            );
            events.extend(start_events);
            if let Some(stop_event) = self.state_manager.handle_content_block_stop(redacted_index) {
                events.push(stop_event);
            }
        }

        // 处理 text 内容：作为 thinking 块的增量
        if let Some(text) = &reasoning.text
            && !text.is_empty()
        {
            // 如果 thinking 块尚未开始，创建它
            if !self.in_thinking_block {
                let thinking_index = self.state_manager.next_block_index();
                self.thinking_block_index = Some(thinking_index);
                self.in_thinking_block = true;
                let start_events = self.state_manager.handle_content_block_start(
                    thinking_index,
                    "thinking",
                    json!({
                            "type": "content_block_start",
                            "index": thinking_index,
                        "content_block": {
                            "type": "thinking",
                            "thinking": ""
                        }
                    }),
                );
                events.extend(start_events);
            }

            // 发送 thinking_delta
            if let Some(thinking_index) = self.thinking_block_index {
                events.push(self.create_thinking_delta_event(thinking_index, text));
            }

            // 估算 tokens
            self.output_tokens += estimate_tokens(text);
        }

        // 处理 signature：暂存，收到 signature 表示 thinking 块即将结束
        if let Some(sig) = &reasoning.signature {
            let mut started_empty_thinking_block = false;
            if !self.in_thinking_block && reasoning.redacted_content.is_none() {
                let thinking_index = self.state_manager.next_block_index();
                self.thinking_block_index = Some(thinking_index);
                self.in_thinking_block = true;
                started_empty_thinking_block = true;
                let start_events = self.state_manager.handle_content_block_start(
                    thinking_index,
                    "thinking",
                    json!({
                            "type": "content_block_start",
                            "index": thinking_index,
                        "content_block": {
                            "type": "thinking",
                            "thinking": ""
                        }
                    }),
                );
                events.extend(start_events);
            }

            self.pending_signature = Some(sig.clone());
            // signature 到达意味着 thinking 块结束，关闭它
            if self.in_thinking_block {
                if let Some(thinking_index) = self.thinking_block_index {
                    if started_empty_thinking_block {
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                    }
                    let normalized_sig = normalize_signature_for_sse(sig, &self.model);
                    events.push(SseEvent::new(
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": thinking_index,
                            "delta": {
                                "type": "signature_delta",
                                "signature": normalized_sig
                            }
                        }),
                    ));
                    if let Some(stop_event) =
                        self.state_manager.handle_content_block_stop(thinking_index)
                    {
                        events.push(stop_event);
                    }
                }
                self.in_thinking_block = false;
                self.thinking_extracted = true;
                self.pending_signature = None; // already emitted
            }
        }

        events
    }

    /// 处理工具使用事件
    fn process_tool_use(
        &mut self,
        tool_use: &crate::kiro::model::events::ToolUseEvent,
    ) -> Vec<SseEvent> {
        let mut events = Vec::new();

        self.state_manager.set_has_tool_use(true);

        // tool_use 必须发生在 thinking 结束之后。
        // 但当 `</thinking>` 后面没有 `\n\n`（例如紧跟 tool_use 或流结束）时，
        // thinking 结束标签会滞留在 thinking_buffer，导致后续 flush 时把 `</thinking>` 当作内容输出。
        // 这里在开始 tool_use block 前做一次"边界场景"的结束标签识别与过滤。
        if self.thinking_enabled
            && self.in_thinking_block
            && let Some(end_pos) = find_real_thinking_end_tag_at_buffer_end(&self.thinking_buffer)
        {
            let thinking_content = self.thinking_buffer[..end_pos].to_string();
            if let Some(thinking_index) = self.thinking_block_index
                && !thinking_content.is_empty()
            {
                events.push(self.create_thinking_delta_event(thinking_index, &thinking_content));
            }

            // 结束 thinking 块
            self.in_thinking_block = false;
            self.thinking_extracted = true;

            if let Some(thinking_index) = self.thinking_block_index {
                // 先发送空的 thinking_delta
                events.push(self.create_thinking_delta_event(thinking_index, ""));
                // 再发送 content_block_stop
                if let Some(stop_event) =
                    self.state_manager.handle_content_block_stop(thinking_index)
                {
                    events.push(stop_event);
                }
            }

            // 把结束标签后的内容当作普通文本（通常为空或空白）
            let after_pos = end_pos + "</thinking>".len();
            let remaining = self.thinking_buffer[after_pos..].trim_start().to_string();
            self.thinking_buffer.clear();
            if !remaining.is_empty() {
                events.extend(self.create_text_delta_events(&remaining));
            }
        }

        // thinking 模式下，process_content_with_thinking 可能会为了探测 `<thinking>` 而暂存一小段尾部文本。
        // 如果此时直接开始 tool_use，状态机会自动关闭 text block，导致这段"待输出文本"看起来被 tool_use 吞掉。
        // 约束：只在尚未进入 thinking block、且 thinking 尚未被提取时，将缓冲区当作普通文本 flush。
        if self.thinking_enabled
            && !self.in_thinking_block
            && !self.thinking_extracted
            && !self.thinking_buffer.is_empty()
        {
            let buffered = std::mem::take(&mut self.thinking_buffer);
            events.extend(self.create_text_delta_events(&buffered));
        }

        let tool_id = ensure_toolu_prefix(&tool_use.tool_use_id);

        // 获取或分配块索引
        let block_index = if let Some(&idx) = self.tool_block_indices.get(&tool_id) {
            idx
        } else {
            let idx = self.state_manager.next_block_index();
            self.tool_block_indices.insert(tool_id.clone(), idx);
            idx
        };

        // 还原工具名称（如果有映射）
        let original_name = self
            .tool_name_map
            .get(&tool_use.name)
            .cloned()
            .unwrap_or_else(|| tool_use.name.clone());

        // 发送 content_block_start
        let start_events = self.state_manager.handle_content_block_start(
            block_index,
            "tool_use",
            json!({
                "type": "content_block_start",
                "index": block_index,
                "content_block": {
                    "type": "tool_use",
                    "id": tool_id,
                    "name": original_name,
                    "input": {}
                }
            }),
        );
        events.extend(start_events);

        // 发送参数增量 (ToolUseEvent.input 是 String 类型)
        if !tool_use.input.is_empty() {
            self.output_tokens += (tool_use.input.len() as i32 + 3) / 4; // 估算 token

            if let Some(delta_event) = self.state_manager.handle_content_block_delta(
                block_index,
                json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": tool_use.input
                    }
                }),
            ) {
                events.push(delta_event);
            }
        }

        // 如果是完整的工具调用（stop=true），发送 content_block_stop
        if tool_use.stop
            && let Some(stop_event) = self.state_manager.handle_content_block_stop(block_index)
        {
            events.push(stop_event);
        }

        events
    }

    /// 生成最终事件序列
    pub fn generate_final_events(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // Flush thinking_buffer 中的剩余内容
        if self.thinking_enabled && !self.thinking_buffer.is_empty() {
            if self.in_thinking_block {
                // 末尾可能残留 `</thinking>`（例如紧跟 tool_use 或流结束），需要在 flush 时过滤掉结束标签。
                if let Some(end_pos) =
                    find_real_thinking_end_tag_at_buffer_end(&self.thinking_buffer)
                {
                    let thinking_content = self.thinking_buffer[..end_pos].to_string();
                    if let Some(thinking_index) = self.thinking_block_index
                        && !thinking_content.is_empty()
                    {
                        events.push(
                            self.create_thinking_delta_event(thinking_index, &thinking_content),
                        );
                    }

                    // 关闭 thinking 块：先发送空的 thinking_delta，再发送 content_block_stop
                    if let Some(thinking_index) = self.thinking_block_index {
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                        if let Some(stop_event) =
                            self.state_manager.handle_content_block_stop(thinking_index)
                        {
                            events.push(stop_event);
                        }
                    }

                    // 把结束标签后的内容当作普通文本（通常为空或空白）
                    let after_pos = end_pos + "</thinking>".len();
                    let remaining = self.thinking_buffer[after_pos..].trim_start().to_string();
                    self.thinking_buffer.clear();
                    self.in_thinking_block = false;
                    self.thinking_extracted = true;
                    if !remaining.is_empty() {
                        events.extend(self.create_text_delta_events(&remaining));
                    }
                } else {
                    // 如果还在 thinking 块内，发送剩余内容作为 thinking_delta
                    if let Some(thinking_index) = self.thinking_block_index {
                        events.push(
                            self.create_thinking_delta_event(thinking_index, &self.thinking_buffer),
                        );
                    }
                    // 关闭 thinking 块：先发送空的 thinking_delta，再发送 content_block_stop
                    if let Some(thinking_index) = self.thinking_block_index {
                        // 先发送空的 thinking_delta
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                        // 再发送 content_block_stop
                        if let Some(stop_event) =
                            self.state_manager.handle_content_block_stop(thinking_index)
                        {
                            events.push(stop_event);
                        }
                    }
                }
            } else {
                // 否则发送剩余内容作为 text_delta
                let buffer_content = self.thinking_buffer.clone();
                events.extend(self.create_text_delta_events(&buffer_content));
            }
            self.thinking_buffer.clear();
        }

        // 如果还有未关闭的 thinking 块（来自 reasoningContentEvent 路径），关闭它
        if self.in_thinking_block {
            if let Some(thinking_index) = self.thinking_block_index {
                // 发送 signature_delta（如果有待发送的签名）
                if let Some(sig) = self.pending_signature.take() {
                    let normalized_sig = normalize_signature_for_sse(&sig, &self.model);
                    events.push(SseEvent::new(
                        "content_block_delta",
                        json!({
                            "type": "content_block_delta",
                            "index": thinking_index,
                            "delta": {
                                "type": "signature_delta",
                                "signature": normalized_sig
                            }
                        }),
                    ));
                }
                // 发送 content_block_stop
                if let Some(stop_event) =
                    self.state_manager.handle_content_block_stop(thinking_index)
                {
                    events.push(stop_event);
                }
            }
            self.in_thinking_block = false;
        }

        // 结构化输出模式：刷出缓冲区，剥离 markdown fence 后作为 text_delta 发送
        if self.structured_output && !self.structured_output_buffer.is_empty() {
            let cleaned = super::structured_output::extract_json_from_response(
                &self.structured_output_buffer,
            );
            events.extend(self.create_text_delta_events(&cleaned));
        }

        // 如果整个流中只产生了 thinking 块，没有 text 也没有 tool_use，
        // 则设置 stop_reason 为 max_tokens（表示模型耗尽了 token 预算在思考上），
        // 并补发一套完整的 text 事件（内容为一个空格），确保 content 数组中有 text 块
        if self.thinking_enabled
            && self.thinking_block_index.is_some()
            && !self.state_manager.has_non_thinking_blocks()
        {
            self.state_manager.set_stop_reason("max_tokens");
            events.extend(self.create_text_delta_events(" "));
        }

        // 始终基于本地估算输入与 cache 统计来生成 usage，
        // 避免因服务端压缩导致上游 token 统计偏低，使客户端误判上下文大小。
        #[cfg(feature = "sensitive-logs")]
        {
            let final_input_tokens = self.input_tokens;
            let billed_input_tokens = self
                .cache_usage
                .map(|cache_usage| {
                    billed_input_tokens(
                        final_input_tokens,
                        cache_usage.cache_creation_input_tokens,
                        cache_usage.cache_read_input_tokens,
                    )
                })
                .unwrap_or(final_input_tokens);

            tracing::info!(
                estimated_input_tokens = self.input_tokens,
                context_input_tokens = ?self.context_input_tokens,
                final_input_tokens,
                output_tokens = self.output_tokens,
                "StreamContext usage: final_input_tokens={} (估算值), billed_input_tokens={}, context_input_tokens={} (上游值), output_tokens={}",
                final_input_tokens,
                billed_input_tokens,
                self.context_input_tokens.map_or("N/A".to_string(), |v| v.to_string()),
                self.output_tokens
            );
        }

        // 生成最终事件
        events.extend(self.state_manager.generate_final_events(FinalUsage {
            output_tokens: self.output_tokens + self.rewrite_extra_output_tokens,
        }));
        events
    }

    /// 检查改写缓冲区是否包含关键词，需要触发改写
    #[allow(dead_code)]
    pub fn needs_rewrite(&self) -> bool {
        if !self.rewrite_enabled || self.rewrite_text_buffer.is_empty() {
            return false;
        }
        super::rewriter::contains_keywords(&self.rewrite_text_buffer, &self.rewrite_keywords)
    }

    /// 取出改写缓冲区内容（消耗）
    pub fn take_rewrite_buffer(&mut self) -> String {
        std::mem::take(&mut self.rewrite_text_buffer)
    }

    /// 将文本作为 text_delta 事件 flush 出去（改写完成后或无需改写时调用）
    pub fn flush_text_as_events(&mut self, text: &str) -> Vec<SseEvent> {
        self.create_text_delta_events(text)
    }

    /// 将改写消耗的额外 output token 加入 usage 计数
    pub fn add_rewrite_tokens(&mut self, extra_output_tokens: i32) {
        self.rewrite_extra_output_tokens += extra_output_tokens;
    }

    /// 将文本拆分为模拟逐字输出的小块，存入 drip 队列
    ///
    /// 按字符边界拆分，每块约 4-12 字符（模拟正常 token 级别的输出粒度）
    pub fn enqueue_drip_text(&mut self, text: &str) {
        use crate::common::utf8::floor_char_boundary;

        if text.is_empty() {
            return;
        }

        // 每块大小：4-12 字节随机，模拟真实 token 输出节奏
        let mut offset = 0;
        while offset < text.len() {
            let chunk_size = 4 + fastrand::usize(..9); // 4-12 字节
            let end = (offset + chunk_size).min(text.len());
            let safe_end = floor_char_boundary(text, end);
            let safe_end = if safe_end <= offset {
                // floor_char_boundary 可能回退到 offset 之前，取下一个完整字符
                text.ceil_char_boundary(offset + 1).min(text.len())
            } else {
                safe_end
            };
            self.rewrite_drip_queue
                .push_back(text[offset..safe_end].to_string());
            offset = safe_end;
        }
    }

    /// 从 drip 队列中弹出下一块文本并生成 text_delta 事件
    pub fn pop_drip_chunk(&mut self) -> Option<Vec<SseEvent>> {
        let chunk = self.rewrite_drip_queue.pop_front()?;
        Some(self.flush_text_as_events(&chunk))
    }

    /// drip 队列是否还有待输出的内容
    #[allow(dead_code)]
    pub fn has_drip_pending(&self) -> bool {
        !self.rewrite_drip_queue.is_empty()
    }
}

/// 将总输入 token 转为 Anthropic usage 的 input_tokens 口径（剔除 cache 读写）
fn billed_input_tokens(
    input_tokens: i32,
    cache_creation_input_tokens: i32,
    cache_read_input_tokens: i32,
) -> i32 {
    input_tokens
        .saturating_sub(cache_creation_input_tokens)
        .saturating_sub(cache_read_input_tokens)
        .max(0)
}

/// 简单的 token 估算
fn estimate_tokens(text: &str) -> i32 {
    let chars: Vec<char> = text.chars().collect();
    let mut chinese_count = 0;
    let mut other_count = 0;

    for c in &chars {
        if *c >= '\u{4E00}' && *c <= '\u{9FFF}' {
            chinese_count += 1;
        } else {
            other_count += 1;
        }
    }

    // 中文约 1.5 字符/token，英文约 4 字符/token
    let chinese_tokens = (chinese_count * 2 + 2) / 3;
    let other_tokens = (other_count + 3) / 4;

    (chinese_tokens + other_tokens).max(1)
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;

    use super::*;

    const OPUS_4_6_NATIVE_SIGNATURE: &str = "EowCClsIDhABGAIqQDw48d35ueoqnU8LHTloJfLtaBDCalB/liNALiPdXRPs/jDhvlOxtZDHNKsi9pUezebhZI7lvXbEUbm5+KrtfrMyD2NsYXVkZS1vcHVzLTQtNjgAEgzWrWaY+VIdCWQ9rOcaDOtHSTHe6lyQ7/kdWSIwFji73XQaiBZfd4DZrpvO9WVIhZdWlnrosB80ah/yuUxoROqeMw3GMs8jOwJzMwvsKl+gXlfdsxNBd+u0oWt/9QL63wCCC3fJ4064uLyRpG8RxfajLTXD0On/b4CMPojf/Y2y1tItPXLydkpmTrNYeURKwPW6B+KPG2Js7M61bCSMK2QDaBJsZWi9o63u+PUEhxgB";
    const SONNET_4_6_NATIVE_SIGNATURE: &str = "EvMDCl0IDhABGAIqQCQxXCPyNpy4NjRc4YfOgFeEfE35bbSttHjk5kBLT+rBEhYCrMFDaLoXQP2fN9DkoM5Knk8Q2CdqNs6mZWWYsd4yEWNsYXVkZS1zb25uZXQtNC02OAASDHviyTd3V5VI9wvGHhoMvH61Ck1wVJisS+RMIjDjgFuB23Eyyqe87awWWRrs2q+rsNp7YsQanBd5qYjDkBGx0Su0S43niVolBmq79joqwwJRWCbrvhFf21GmNh/HlUpDhb40k8birhtaNUyBofflJJKGmVYYhipCp/N7qtjN08WeWG8SkjzQKHz9fRRRgCAe+Jo+YNe61crCk9svmVaaU2BDDVspdeYYgP01Nx6qE+YB1njr39/NDOMopBa6Jn7TShkuZRVm4NsJ1x1B31kWKPsNBa8LnwZzAKfjv0BtgiBJd7qzm9iqUebkOWxKAUAJc/cw/XzmN5dZDol2cIHlBYmROc6Rjp8qzJ4NRXBLeIlllHzE0GHyhzU/hhQCKRG9oofJ6UtbrRv+/0YZq8/adNlpFJ93IbrMkv2WEdcPXsRb1/70BGrnHd5Op+GZ/7vbCRpK3s0SwHUGygudAcqlGk329HqZqMZ/KnOKwdYarcwjvn/Efh+tx0IWW/4BUv6lxlvy8DUpOa4CuQqfuB4DjXjc9xgB";

    fn decode_signature(signature: &str) -> Vec<u8> {
        STANDARD
            .decode(signature)
            .expect("signature should be base64")
    }

    fn signature_with_model_marker(signature: &str, from: &[u8], to: &[u8]) -> String {
        assert_eq!(
            from.len(),
            to.len(),
            "replacement model marker must preserve length"
        );
        let mut raw = decode_signature(signature);
        let pos = raw
            .windows(from.len())
            .position(|window| window == from)
            .expect("source model marker should exist");
        raw[pos..pos + to.len()].copy_from_slice(to);
        STANDARD.encode(raw)
    }

    fn signature_with_existing_thinking_field(signature: &str) -> String {
        let raw = decode_signature(signature);
        let (outer_len, outer_start) = read_varint_after_tag(&raw, 0, 0x12).unwrap();
        let outer_end = outer_start + outer_len;
        let (meta_len, meta_start) = read_varint_after_tag(&raw, outer_start, 0x0a).unwrap();
        let meta_end = meta_start + meta_len;

        let mut meta = raw[meta_start..meta_end].to_vec();
        let (_, insert_pos) = find_varint_field(&meta, 7, 0).unwrap();
        meta.splice(insert_pos..insert_pos, b"\x42\x08thinking".iter().copied());

        let mut rebuilt_outer = Vec::new();
        rebuilt_outer.push(0x0a);
        write_varint(meta.len(), &mut rebuilt_outer);
        rebuilt_outer.extend_from_slice(&meta);
        rebuilt_outer.extend_from_slice(&raw[meta_end..outer_end]);

        let mut rebuilt = Vec::new();
        rebuilt.push(0x12);
        write_varint(rebuilt_outer.len(), &mut rebuilt);
        rebuilt.extend_from_slice(&rebuilt_outer);
        rebuilt.extend_from_slice(&raw[outer_end..]);

        STANDARD.encode(rebuilt)
    }

    fn signature_with_model_field(signature: &str, model: &[u8]) -> String {
        let raw = decode_signature(signature);
        let (outer_len, outer_start) = read_varint_after_tag(&raw, 0, 0x12).unwrap();
        let outer_end = outer_start + outer_len;
        let (meta_len, meta_start) = read_varint_after_tag(&raw, outer_start, 0x0a).unwrap();
        let meta_end = meta_start + meta_len;

        let mut meta = raw[meta_start..meta_end].to_vec();
        let (len_start, _payload_start, payload_end) =
            find_test_length_delimited_field(&meta, 6).unwrap();
        let mut replacement = Vec::new();
        write_varint(model.len(), &mut replacement);
        replacement.extend_from_slice(model);
        meta.splice(len_start..payload_end, replacement);

        let mut rebuilt_outer = Vec::new();
        rebuilt_outer.push(0x0a);
        write_varint(meta.len(), &mut rebuilt_outer);
        rebuilt_outer.extend_from_slice(&meta);
        rebuilt_outer.extend_from_slice(&raw[meta_end..outer_end]);

        let mut rebuilt = Vec::new();
        rebuilt.push(0x12);
        write_varint(rebuilt_outer.len(), &mut rebuilt);
        rebuilt.extend_from_slice(&rebuilt_outer);
        rebuilt.extend_from_slice(&raw[outer_end..]);

        STANDARD.encode(rebuilt)
    }

    fn signature_with_extra_model_field(signature: &str, model: &[u8]) -> String {
        let raw = decode_signature(signature);
        let (outer_len, outer_start) = read_varint_after_tag(&raw, 0, 0x12).unwrap();
        let outer_end = outer_start + outer_len;
        let (meta_len, meta_start) = read_varint_after_tag(&raw, outer_start, 0x0a).unwrap();
        let meta_end = meta_start + meta_len;

        let mut meta = raw[meta_start..meta_end].to_vec();
        let mut decoy = vec![0x32];
        write_varint(model.len(), &mut decoy);
        decoy.extend_from_slice(model);
        meta.splice(0..0, decoy);

        let mut rebuilt_outer = Vec::new();
        rebuilt_outer.push(0x0a);
        write_varint(meta.len(), &mut rebuilt_outer);
        rebuilt_outer.extend_from_slice(&meta);
        rebuilt_outer.extend_from_slice(&raw[meta_end..outer_end]);

        let mut rebuilt = Vec::new();
        rebuilt.push(0x12);
        write_varint(rebuilt_outer.len(), &mut rebuilt);
        rebuilt.extend_from_slice(&rebuilt_outer);
        rebuilt.extend_from_slice(&raw[outer_end..]);

        STANDARD.encode(rebuilt)
    }

    fn find_test_length_delimited_field(
        data: &[u8],
        target_field: usize,
    ) -> Option<(usize, usize, usize)> {
        let mut pos = 0;
        while pos < data.len() {
            let (key, value_start) = read_varint(data, pos)?;
            let field_number = key >> 3;
            let wire_type = key & 0x07;
            pos = value_start;

            match wire_type {
                0 => {
                    let (_, field_end) = read_varint(data, pos)?;
                    pos = field_end;
                }
                1 => pos = pos.checked_add(8)?,
                2 => {
                    let len_start = pos;
                    let (len, payload_start) = read_varint(data, pos)?;
                    let payload_end = payload_start.checked_add(len)?;
                    if payload_end > data.len() {
                        return None;
                    }
                    if field_number == target_field {
                        return Some((len_start, payload_start, payload_end));
                    }
                    pos = payload_end;
                }
                5 => pos = pos.checked_add(4)?,
                _ => return None,
            }
        }
        None
    }

    fn contains_native_marker(raw: &[u8]) -> bool {
        raw.windows([0x10, 0x01, 0x18, 0x02].len())
            .any(|w| w == [0x10, 0x01, 0x18, 0x02])
    }

    fn count_marker(raw: &[u8], marker: &[u8]) -> usize {
        raw.windows(marker.len())
            .filter(|window| *window == marker)
            .count()
    }

    #[test]
    fn normalizes_opus_4_6_native_signature_to_thinking_marker() {
        let raw = decode_signature(OPUS_4_6_NATIVE_SIGNATURE);
        assert!(
            raw.windows(b"claude-opus-4-6".len())
                .any(|w| w == b"claude-opus-4-6")
        );
        assert!(!raw.windows(b"thinking".len()).any(|w| w == b"thinking"));
        assert!(
            raw.windows([0x10, 0x01, 0x18, 0x02].len())
                .any(|w| w == [0x10, 0x01, 0x18, 0x02])
        );

        let normalized = normalize_thinking_signature(OPUS_4_6_NATIVE_SIGNATURE, false)
            .expect("opus 4.6 native signature should normalize");
        let normalized_raw = decode_signature(&normalized);

        assert!(
            normalized_raw
                .windows(b"claude-opus-4-6".len())
                .any(|w| w == b"claude-opus-4-6")
        );
        assert!(
            normalized_raw
                .windows(b"thinking".len())
                .any(|w| w == b"thinking")
        );
        assert!(
            !normalized_raw
                .windows([0x10, 0x01, 0x18, 0x02].len())
                .any(|w| w == [0x10, 0x01, 0x18, 0x02])
        );
    }

    #[test]
    fn normalizes_signature_for_sse_only_for_4_6_models() {
        assert_eq!(
            normalize_signature_for_sse(OPUS_4_6_NATIVE_SIGNATURE, "claude-opus-4-7"),
            OPUS_4_6_NATIVE_SIGNATURE
        );
        assert_eq!(
            normalize_signature_for_sse(OPUS_4_6_NATIVE_SIGNATURE, "claude-sonnet-4-5"),
            OPUS_4_6_NATIVE_SIGNATURE
        );

        let normalized =
            normalize_signature_for_sse(OPUS_4_6_NATIVE_SIGNATURE, "claude-opus-4-6-thinking");
        assert_ne!(normalized, OPUS_4_6_NATIVE_SIGNATURE);
        assert!(
            decode_signature(&normalized)
                .windows(b"thinking".len())
                .any(|w| w == b"thinking")
        );
    }

    #[test]
    fn normalizes_sonnet_4_6_native_signature_to_thinking_marker() {
        let normalized =
            normalize_signature_for_sse(SONNET_4_6_NATIVE_SIGNATURE, "claude-sonnet-4-6");
        assert_ne!(normalized, SONNET_4_6_NATIVE_SIGNATURE);

        let normalized_raw = decode_signature(&normalized);
        assert!(
            normalized_raw
                .windows(b"claude-sonnet-4-6".len())
                .any(|w| w == b"claude-sonnet-4-6")
        );
        assert!(
            normalized_raw
                .windows(b"thinking".len())
                .any(|w| w == b"thinking")
        );
        assert!(
            !normalized_raw
                .windows([0x10, 0x01, 0x18, 0x02].len())
                .any(|w| w == [0x10, 0x01, 0x18, 0x02])
        );
    }

    #[test]
    fn normalizes_native_signature_for_thinking_suffix_models() {
        let opus_4_7_signature = signature_with_model_marker(
            OPUS_4_6_NATIVE_SIGNATURE,
            b"claude-opus-4-6",
            b"claude-opus-4-7",
        );

        for request_model in ["claude-opus-4-7", "claude-opus-4-7-thinking"] {
            let normalized = normalize_signature_for_sse(&opus_4_7_signature, request_model);
            assert_ne!(normalized, opus_4_7_signature);

            let normalized_raw = decode_signature(&normalized);
            assert!(
                normalized_raw
                    .windows(b"claude-opus-4-7".len())
                    .any(|w| w == b"claude-opus-4-7")
            );
            assert!(
                normalized_raw
                    .windows(b"thinking".len())
                    .any(|w| w == b"thinking")
            );
            assert!(
                !normalized_raw
                    .windows([0x10, 0x01, 0x18, 0x02].len())
                    .any(|w| w == [0x10, 0x01, 0x18, 0x02])
            );
        }
    }

    #[test]
    fn normalizes_existing_thinking_signature_for_base_models_without_duplication() {
        let opus_4_7_signature = signature_with_model_marker(
            OPUS_4_6_NATIVE_SIGNATURE,
            b"claude-opus-4-6",
            b"claude-opus-4-7",
        );
        let dual_marker_signature = signature_with_existing_thinking_field(&opus_4_7_signature);
        let dual_marker_raw = decode_signature(&dual_marker_signature);
        assert!(contains_native_marker(&dual_marker_raw));
        assert_eq!(count_marker(&dual_marker_raw, b"thinking"), 1);

        let normalized = normalize_signature_for_sse(&dual_marker_signature, "claude-opus-4-7");
        let normalized_raw = decode_signature(&normalized);

        assert!(!contains_native_marker(&normalized_raw));
        assert_eq!(count_marker(&normalized_raw, b"thinking"), 1);
    }

    #[test]
    fn normalizes_quince_alias_to_external_model_marker() {
        for (alias, external_model) in [
            (b"claude-quince7".as_slice(), "claude-opus-4-7"),
            (b"claude-quince8".as_slice(), "claude-opus-4-8"),
            (b"claude-quince".as_slice(), "claude-opus-4-7"),
            (b"claude-quince".as_slice(), "claude-opus-4-8"),
        ] {
            let quince_signature = signature_with_model_field(OPUS_4_6_NATIVE_SIGNATURE, alias);
            let dual_marker_signature = signature_with_extra_model_field(
                &signature_with_extra_model_field(
                    &signature_with_existing_thinking_field(&quince_signature),
                    b"claude-decoy",
                ),
                alias,
            );
            let dual_marker_raw = decode_signature(&dual_marker_signature);
            assert!(dual_marker_raw.windows(alias.len()).any(|w| w == alias));
            assert!(
                dual_marker_raw
                    .windows(b"claude-decoy".len())
                    .any(|w| w == b"claude-decoy")
            );
            assert!(contains_native_marker(&dual_marker_raw));
            assert_eq!(count_marker(&dual_marker_raw, b"thinking"), 1);

            let normalized = normalize_signature_for_sse(&dual_marker_signature, external_model);
            let normalized_raw = decode_signature(&normalized);

            assert!(
                normalized_raw
                    .windows(external_model.len())
                    .any(|w| w == external_model.as_bytes())
            );
            assert!(!normalized_raw.windows(alias.len()).any(|w| w == alias));
            assert!(!contains_native_marker(&normalized_raw));
            assert_eq!(count_marker(&normalized_raw, b"thinking"), 1);
        }
    }

    #[test]
    fn normalizes_quince_alias_for_public_model_variants() {
        for (alias, request_model, external_model) in [
            (
                b"claude-quince7".as_slice(),
                "claude-opus-4-7-thinking",
                "claude-opus-4-7",
            ),
            (
                b"claude-quince7".as_slice(),
                "claude-opus-4-7-agentic",
                "claude-opus-4-7",
            ),
            (
                b"claude-quince7".as_slice(),
                "claude-opus-4.7",
                "claude-opus-4-7",
            ),
            (
                b"claude-quince8".as_slice(),
                "claude-opus-4-8-thinking",
                "claude-opus-4-8",
            ),
            (
                b"claude-quince8".as_slice(),
                "claude-opus-4-8-agentic",
                "claude-opus-4-8",
            ),
            (
                b"claude-quince8".as_slice(),
                "claude-opus-4.8",
                "claude-opus-4-8",
            ),
        ] {
            let quince_signature = signature_with_model_field(OPUS_4_6_NATIVE_SIGNATURE, alias);
            let signature = signature_with_existing_thinking_field(&quince_signature);

            let normalized = normalize_signature_for_sse(&signature, request_model);
            let normalized_raw = decode_signature(&normalized);

            assert!(
                normalized_raw
                    .windows(external_model.len())
                    .any(|w| w == external_model.as_bytes())
            );
            assert!(!normalized_raw.windows(alias.len()).any(|w| w == alias));
            assert!(!contains_native_marker(&normalized_raw));
            assert_eq!(count_marker(&normalized_raw, b"thinking"), 1);
        }
    }

    #[test]
    fn normalizes_native_only_quince_alias_for_base_models() {
        for (alias, request_model, external_model) in [
            (
                b"claude-quince7".as_slice(),
                "claude-opus-4-7",
                "claude-opus-4-7",
            ),
            (
                b"claude-quince8".as_slice(),
                "claude-opus-4-8",
                "claude-opus-4-8",
            ),
        ] {
            let signature = signature_with_model_field(OPUS_4_6_NATIVE_SIGNATURE, alias);
            let raw = decode_signature(&signature);
            assert!(raw.windows(alias.len()).any(|w| w == alias));
            assert!(contains_native_marker(&raw));
            assert_eq!(count_marker(&raw, b"thinking"), 0);

            let normalized = normalize_signature_for_sse(&signature, request_model);
            let normalized_raw = decode_signature(&normalized);

            assert!(
                normalized_raw
                    .windows(external_model.len())
                    .any(|w| w == external_model.as_bytes())
            );
            assert!(!normalized_raw.windows(alias.len()).any(|w| w == alias));
            assert!(!contains_native_marker(&normalized_raw));
            assert_eq!(count_marker(&normalized_raw, b"thinking"), 1);
        }
    }

    fn zero_cache_usage() -> Option<CacheUsageBreakdown> {
        Some(CacheUsageBreakdown {
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            cache_creation_5m_input_tokens: 0,
            cache_creation_1h_input_tokens: 0,
        })
    }

    fn cache_usage(
        cache_creation_input_tokens: i32,
        cache_read_input_tokens: i32,
        cache_creation_5m_input_tokens: i32,
        cache_creation_1h_input_tokens: i32,
    ) -> Option<CacheUsageBreakdown> {
        Some(CacheUsageBreakdown {
            cache_creation_input_tokens,
            cache_read_input_tokens,
            cache_creation_5m_input_tokens,
            cache_creation_1h_input_tokens,
        })
    }

    #[test]
    fn test_sse_event_format() {
        let event = SseEvent::new("message_start", json!({"type": "message_start"}));
        let sse_str = event.to_sse_string();

        assert!(sse_str.starts_with("event: message_start\n"));
        assert!(sse_str.contains("data: "));
        assert!(sse_str.ends_with("\n\n"));
    }

    #[test]
    fn test_sse_state_manager_message_start() {
        let mut manager = SseStateManager::new();

        // 第一次应该成功
        let event = manager.handle_message_start(json!({"type": "message_start"}));
        assert!(event.is_some());

        // 第二次应该被跳过
        let event = manager.handle_message_start(json!({"type": "message_start"}));
        assert!(event.is_none());
    }

    #[test]
    fn test_sse_state_manager_block_lifecycle() {
        let mut manager = SseStateManager::new();

        // 创建块
        let events = manager.handle_content_block_start(0, "text", json!({}));
        assert_eq!(events.len(), 1);

        // delta
        let event = manager.handle_content_block_delta(0, json!({}));
        assert!(event.is_some());

        // stop
        let event = manager.handle_content_block_stop(0);
        assert!(event.is_some());

        // 重复 stop 应该被跳过
        let event = manager.handle_content_block_stop(0);
        assert!(event.is_none());
    }

    #[test]
    fn test_stream_context_message_delta_only_has_output_tokens() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            123,
            zero_cache_usage(),
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let initial_events = ctx.generate_initial_events();
        let mut all_events = initial_events;
        ctx.process_kiro_event(&Event::Metering(MeteringEvent {
            unit: "credit".to_string(),
            unit_plural: "credits".to_string(),
            usage: 0.75,
        }));

        all_events.extend(ctx.generate_final_events());
        let message_start_usage = all_events
            .iter()
            .find(|e| e.event == "message_start")
            .map(|e| e.data["message"]["usage"].clone())
            .expect("message_start should exist");

        let message_delta_usage = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .map(|e| e.data["usage"].clone())
            .expect("message_delta should exist");

        // message_start should have input_tokens and cache fields
        assert_eq!(message_start_usage["input_tokens"], json!(123));
        assert_eq!(message_start_usage["cache_creation_input_tokens"], json!(0));
        assert_eq!(message_start_usage["cache_read_input_tokens"], json!(0));

        // message_delta.usage should ONLY have output_tokens (per Anthropic spec)
        assert!(message_delta_usage.get("output_tokens").is_some());
        assert!(message_delta_usage.get("input_tokens").is_none());
        assert!(
            message_delta_usage
                .get("cache_creation_input_tokens")
                .is_none()
        );
        assert!(message_delta_usage.get("cache_read_input_tokens").is_none());
        assert!(message_delta_usage.get("credit_usage").is_none());
        assert!(message_delta_usage.get("credit_unit").is_none());
        assert!(message_delta_usage.get("credit_unit_plural").is_none());
    }

    #[test]
    fn test_stream_context_includes_cache_usage_fields() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            321,
            cache_usage(7, 8, 0, 0),
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let all_events = ctx.generate_initial_events();

        let message_start_usage = all_events
            .iter()
            .find(|e| e.event == "message_start")
            .map(|e| e.data["message"]["usage"].clone())
            .expect("message_start should exist");

        assert_eq!(message_start_usage["input_tokens"], json!(306));
        assert_eq!(message_start_usage["cache_creation_input_tokens"], json!(7));
        assert_eq!(message_start_usage["cache_read_input_tokens"], json!(8));
    }
    #[test]
    fn test_stream_context_uses_billed_input_tokens_when_cache_read_present() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            321,
            cache_usage(7, 8, 0, 0),
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let initial_events = ctx.generate_initial_events();
        let message_start_usage = initial_events
            .iter()
            .find(|e| e.event == "message_start")
            .map(|e| e.data["message"]["usage"].clone())
            .expect("message_start should exist");

        let final_events = ctx.generate_final_events();
        let message_delta_usage = final_events
            .iter()
            .find(|e| e.event == "message_delta")
            .map(|e| e.data["usage"].clone())
            .expect("message_delta should exist");

        assert_eq!(message_start_usage["input_tokens"], json!(306));
        // message_delta only has output_tokens
        assert!(message_delta_usage.get("input_tokens").is_none());
        assert!(
            message_delta_usage
                .get("cache_creation_input_tokens")
                .is_none()
        );
        assert!(message_delta_usage.get("cache_read_input_tokens").is_none());
        assert!(message_delta_usage.get("output_tokens").is_some());
    }
    #[test]
    fn test_stream_context_omits_cache_usage_fields_when_disabled() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            321,
            None,
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let initial_events = ctx.generate_initial_events();
        let message_start_usage = initial_events
            .iter()
            .find(|e| e.event == "message_start")
            .map(|e| e.data["message"]["usage"].clone())
            .expect("message_start should exist");

        let final_events = ctx.generate_final_events();
        let message_delta_usage = final_events
            .iter()
            .find(|e| e.event == "message_delta")
            .map(|e| e.data["usage"].clone())
            .expect("message_delta should exist");

        assert_eq!(message_start_usage["input_tokens"], json!(321));
        assert!(
            message_start_usage
                .get("cache_creation_input_tokens")
                .is_none()
        );
        assert!(message_start_usage.get("cache_read_input_tokens").is_none());
        // message_delta only has output_tokens regardless of cache setting
        assert!(message_delta_usage.get("output_tokens").is_some());
        assert!(message_delta_usage.get("input_tokens").is_none());
        assert!(
            message_delta_usage
                .get("cache_creation_input_tokens")
                .is_none()
        );
        assert!(message_delta_usage.get("cache_read_input_tokens").is_none());
        assert!(message_delta_usage.get("cache_creation").is_none());
    }

    #[test]
    fn test_stream_context_cache_in_message_start_not_delta() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1000,
            cache_usage(50, 800, 30, 20),
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );

        ctx.process_kiro_event(&Event::Metering(MeteringEvent {
            unit: "credit".to_string(),
            unit_plural: "credits".to_string(),
            usage: 0.5,
        }));

        let initial_events = ctx.generate_initial_events();
        let message_start_usage = initial_events
            .iter()
            .find(|e| e.event == "message_start")
            .map(|e| e.data["message"]["usage"].clone())
            .expect("message_start should exist");

        // message_start should have cache fields
        assert_eq!(message_start_usage["cache_read_input_tokens"], json!(800));
        assert_eq!(
            message_start_usage["cache_creation_input_tokens"],
            json!(50)
        );
        // billed input = 1000 - 50 - 800 = 150
        assert_eq!(message_start_usage["input_tokens"], json!(150));

        let final_events = ctx.generate_final_events();
        let message_delta_usage = final_events
            .iter()
            .find(|e| e.event == "message_delta")
            .map(|e| e.data["usage"].clone())
            .expect("message_delta should exist");

        // message_delta should only have output_tokens
        assert!(message_delta_usage.get("output_tokens").is_some());
        assert!(message_delta_usage.get("input_tokens").is_none());
        assert!(message_delta_usage.get("cache_read_input_tokens").is_none());
        assert!(
            message_delta_usage
                .get("cache_creation_input_tokens")
                .is_none()
        );
        assert!(message_delta_usage.get("cache_creation").is_none());
        assert!(message_delta_usage.get("credit_usage").is_none());
    }

    #[test]
    fn test_text_delta_after_tool_use_restarts_text_block() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );

        let initial_events = ctx.generate_initial_events();
        assert!(
            initial_events
                .iter()
                .all(|e| !(e.event == "content_block_start"
                    && e.data["content_block"]["type"] == "text"))
        );

        // 首次输出文本时应创建 text block
        let first_text_events = ctx.process_assistant_response("hi");
        let initial_text_index = first_text_events
            .iter()
            .find_map(|e| {
                if e.event == "content_block_start" && e.data["content_block"]["type"] == "text" {
                    e.data["index"].as_i64()
                } else {
                    None
                }
            })
            .expect("text block should start on first text delta")
            as i32;

        // tool_use 开始会自动关闭现有 text block
        let tool_events = ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
            name: "test_tool".to_string(),
            tool_use_id: "tool_1".to_string(),
            input: "{}".to_string(),
            stop: false,
        });
        assert!(
            tool_events.iter().any(|e| {
                e.event == "content_block_stop"
                    && e.data["index"].as_i64() == Some(initial_text_index as i64)
            }),
            "tool_use should stop the previous text block"
        );

        // 之后再来文本增量，应自动创建新的 text block 而不是往已 stop 的块里写 delta
        let text_events = ctx.process_assistant_response("hello");
        let new_text_start_index = text_events.iter().find_map(|e| {
            if e.event == "content_block_start" && e.data["content_block"]["type"] == "text" {
                e.data["index"].as_i64()
            } else {
                None
            }
        });
        assert!(
            new_text_start_index.is_some(),
            "should start a new text block"
        );
        assert_ne!(
            new_text_start_index.unwrap(),
            initial_text_index as i64,
            "new text block index should differ from the stopped one"
        );
        assert!(
            text_events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "text_delta"
                    && e.data["delta"]["text"] == "hello"
            }),
            "should emit text_delta after restarting text block"
        );
    }

    #[test]
    fn test_tool_use_only_does_not_emit_empty_text_block() {
        // tool_use-only 的流式响应不应产生空 text block（text=""），否则客户端写回 history 会触发上游校验拒绝
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );

        let mut all_events = Vec::new();
        all_events.extend(ctx.generate_initial_events());
        all_events.extend(
            ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
                name: "test_tool".to_string(),
                tool_use_id: "tool_1".to_string(),
                input: "{}".to_string(),
                stop: true,
            }),
        );
        all_events.extend(ctx.generate_final_events());

        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "tool_use"
            }),
            "should emit tool_use content_block_start"
        );
        assert!(
            all_events.iter().all(|e| {
                !(e.event == "content_block_start" && e.data["content_block"]["type"] == "text")
            }),
            "tool_use-only stream should not start a text block"
        );
    }

    #[test]
    fn test_estimate_tokens() {
        assert!(estimate_tokens("Hello") > 0);
        assert!(estimate_tokens("你好") > 0);
        assert!(estimate_tokens("Hello 你好") > 0);
    }

    #[test]
    fn test_reasoning_content_text_starts_thinking_block() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("Let me think...".to_string()),
            signature: None,
            redacted_content: None,
        }));

        // Should start a thinking block
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "thinking"
            }),
            "should emit thinking content_block_start"
        );

        // Should emit thinking_delta
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "thinking_delta"
                    && e.data["delta"]["thinking"] == "Let me think..."
            }),
            "should emit thinking_delta with text"
        );
    }

    #[test]
    fn test_reasoning_content_signature_closes_thinking_block() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        // First send text to open thinking block
        ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("thinking content".to_string()),
            signature: None,
            redacted_content: None,
        }));

        // Then send signature to close it
        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: None,
            signature: Some("sig_abc123".to_string()),
            redacted_content: None,
        }));

        // Should emit signature_delta
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "signature_delta"
                    && e.data["delta"]["signature"] == "sig_abc123"
            }),
            "should emit signature_delta"
        );

        // Should emit content_block_stop
        assert!(
            events.iter().any(|e| e.event == "content_block_stop"),
            "should emit content_block_stop for thinking block"
        );

        // Thinking block should be closed
        assert!(!ctx.in_thinking_block, "thinking block should be closed");
    }

    #[test]
    fn test_reasoning_content_signature_only_starts_thinking_block() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some(String::new()),
            signature: Some("sig_only".to_string()),
            redacted_content: None,
        }));

        let event_sequence: Vec<String> = events
            .iter()
            .filter_map(|e| match e.event.as_str() {
                "content_block_start" => Some(format!(
                    "content_block_start:{}",
                    e.data["content_block"]["type"].as_str().unwrap_or("")
                )),
                "content_block_delta" => Some(format!(
                    "content_block_delta:{}",
                    e.data["delta"]["type"].as_str().unwrap_or("")
                )),
                "content_block_stop" => Some("content_block_stop".to_string()),
                _ => None,
            })
            .collect();

        assert_eq!(
            event_sequence,
            vec![
                "content_block_start:thinking",
                "content_block_delta:thinking_delta",
                "content_block_delta:signature_delta",
                "content_block_stop",
            ],
            "signature-only thinking should still include an empty thinking_delta before signature_delta"
        );

        assert!(
            events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "thinking"
            }),
            "should emit thinking content_block_start even when text is empty"
        );
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "signature_delta"
                    && e.data["delta"]["signature"] == "sig_only"
            }),
            "should emit signature_delta when signature arrives without text"
        );
        assert!(
            events.iter().any(|e| e.event == "content_block_stop"),
            "should close the thinking block when signature arrives without text"
        );
    }

    #[test]
    fn test_reasoning_content_emits_normalized_opus_4_6_signature() {
        let mut ctx = StreamContext::new_with_thinking(
            "claude-opus-4-6",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("thinking content".to_string()),
            signature: None,
            redacted_content: None,
        }));

        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: None,
            signature: Some(OPUS_4_6_NATIVE_SIGNATURE.to_string()),
            redacted_content: None,
        }));

        let emitted_sig = events
            .iter()
            .find(|event| {
                event.event == "content_block_delta"
                    && event.data["delta"]["type"] == "signature_delta"
            })
            .and_then(|event| event.data["delta"]["signature"].as_str())
            .expect("signature_delta should be emitted");
        let emitted_raw = decode_signature(emitted_sig);

        assert!(
            emitted_raw
                .windows(b"thinking".len())
                .any(|w| w == b"thinking")
        );
        assert!(
            !emitted_raw
                .windows([0x10, 0x01, 0x18, 0x02].len())
                .any(|w| w == [0x10, 0x01, 0x18, 0x02])
        );
    }

    #[test]
    fn test_reasoning_content_redacted_thinking() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: None,
            signature: None,
            redacted_content: Some("encrypted_data_here".to_string()),
        }));

        // Should emit redacted_thinking content_block_start
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_start"
                    && e.data["content_block"]["type"] == "redacted_thinking"
                    && e.data["content_block"]["data"] == "encrypted_data_here"
            }),
            "should emit redacted_thinking content_block_start"
        );

        // Should emit content_block_stop for the redacted block
        assert!(
            events.iter().any(|e| e.event == "content_block_stop"),
            "should emit content_block_stop for redacted_thinking block"
        );
    }

    #[test]
    fn test_reasoning_content_redacted_closes_open_thinking_block() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        // Open a thinking block
        ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("thinking...".to_string()),
            signature: None,
            redacted_content: None,
        }));
        assert!(ctx.in_thinking_block);

        // Send redacted content - should close the thinking block first
        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: None,
            signature: None,
            redacted_content: Some("redacted_data".to_string()),
        }));

        // Should have content_block_stop for the thinking block
        let thinking_index = ctx.thinking_block_index.unwrap();
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_stop"
                    && e.data["index"].as_i64() == Some(thinking_index as i64)
            }),
            "should stop the thinking block before emitting redacted_thinking"
        );

        // Should also have redacted_thinking block
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_start"
                    && e.data["content_block"]["type"] == "redacted_thinking"
            }),
            "should emit redacted_thinking block"
        );
    }

    #[test]
    fn test_reasoning_then_text_response() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();

        // Reasoning content
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: Some("Let me think".to_string()),
                signature: None,
                redacted_content: None,
            },
        )));

        // Close thinking with signature
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: None,
                signature: Some("sig_xyz".to_string()),
                redacted_content: None,
            },
        )));

        // Then text response
        all_events.extend(ctx.process_kiro_event(&Event::AssistantResponse(
            serde_json::from_value(json!({"content": "Hello!"})).unwrap(),
        )));

        all_events.extend(ctx.generate_final_events());

        // Should have thinking block
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "thinking"
            }),
            "should have thinking block"
        );

        // Should have text block
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "text"
            }),
            "should have text block"
        );

        // stop_reason should be end_turn
        let message_delta = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .expect("should have message_delta event");
        assert_eq!(
            message_delta.data["delta"]["stop_reason"], "end_turn",
            "stop_reason should be end_turn when text is also produced"
        );
    }

    #[test]
    fn test_thinking_only_sets_max_tokens_stop_reason() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();

        // Only reasoning content, then signature to close
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: Some("thinking only".to_string()),
                signature: None,
                redacted_content: None,
            },
        )));
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: None,
                signature: Some("sig_end".to_string()),
                redacted_content: None,
            },
        )));

        all_events.extend(ctx.generate_final_events());

        let message_delta = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .expect("should have message_delta event");

        assert_eq!(
            message_delta.data["delta"]["stop_reason"], "max_tokens",
            "stop_reason should be max_tokens when only thinking is produced"
        );

        // Should emit a text block with a single space
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "text"
            }),
            "should emit text content_block_start"
        );
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "text_delta"
                    && e.data["delta"]["text"] == " "
            }),
            "should emit text_delta with a single space"
        );
    }

    #[test]
    fn test_thinking_with_tool_use_keeps_tool_use_stop_reason() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();

        // Reasoning content + signature
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: Some("thinking".to_string()),
                signature: None,
                redacted_content: None,
            },
        )));
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: None,
                signature: Some("sig_tool".to_string()),
                redacted_content: None,
            },
        )));

        // Then tool_use
        all_events.extend(
            ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
                name: "test_tool".to_string(),
                tool_use_id: "tool_1".to_string(),
                input: "{}".to_string(),
                stop: true,
            }),
        );
        all_events.extend(ctx.generate_final_events());

        let message_delta = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .expect("should have message_delta event");

        assert_eq!(
            message_delta.data["delta"]["stop_reason"], "tool_use",
            "stop_reason should be tool_use when tool_use is present"
        );
    }

    #[test]
    fn test_generate_final_events_closes_open_thinking_block_with_signature() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        // Open thinking block but don't close it (no signature received)
        ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("still thinking...".to_string()),
            signature: None,
            redacted_content: None,
        }));
        assert!(ctx.in_thinking_block);

        // Set a pending signature manually to test flush behavior
        ctx.pending_signature = Some("sig_flush".to_string());

        let final_events = ctx.generate_final_events();

        // Should emit signature_delta
        assert!(
            final_events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "signature_delta"
                    && e.data["delta"]["signature"] == "sig_flush"
            }),
            "should emit signature_delta during final flush"
        );

        // Should emit content_block_stop
        assert!(
            final_events.iter().any(|e| e.event == "content_block_stop"),
            "should emit content_block_stop during final flush"
        );
    }

    /// 辅助函数：从事件列表中提取所有 thinking_delta 的拼接内容
    fn collect_thinking_content(events: &[SseEvent]) -> String {
        events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "thinking_delta"
            })
            .map(|e| e.data["delta"]["thinking"].as_str().unwrap_or(""))
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// 辅助函数：从事件列表中提取所有 text_delta 的拼接内容
    #[allow(dead_code)]
    fn collect_text_content(events: &[SseEvent]) -> String {
        events
            .iter()
            .filter(|e| e.event == "content_block_delta" && e.data["delta"]["type"] == "text_delta")
            .map(|e| e.data["delta"]["text"].as_str().unwrap_or(""))
            .collect()
    }

    #[test]
    fn test_multiple_reasoning_chunks() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();

        // Multiple reasoning text chunks
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: Some("First ".to_string()),
                signature: None,
                redacted_content: None,
            },
        )));
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: Some("Second ".to_string()),
                signature: None,
                redacted_content: None,
            },
        )));
        all_events.extend(ctx.process_kiro_event(&Event::ReasoningContent(
            ReasoningContentEvent {
                text: Some("Third".to_string()),
                signature: None,
                redacted_content: None,
            },
        )));

        let thinking = collect_thinking_content(&all_events);
        assert_eq!(thinking, "First Second Third");

        // Only one content_block_start for thinking
        let thinking_starts: Vec<_> = all_events
            .iter()
            .filter(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "thinking"
            })
            .collect();
        assert_eq!(
            thinking_starts.len(),
            1,
            "should only start one thinking block"
        );
    }

    #[test]
    fn test_reasoning_with_text_and_signature_in_same_event() {
        let mut ctx = StreamContext::new_with_thinking(
            "test-model",
            1,
            zero_cache_usage(),
            true,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let _initial_events = ctx.generate_initial_events();

        // First open the thinking block
        ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("initial thought".to_string()),
            signature: None,
            redacted_content: None,
        }));

        // Event with empty text and signature (signals end)
        let events = ctx.process_kiro_event(&Event::ReasoningContent(ReasoningContentEvent {
            text: Some("".to_string()),
            signature: Some("sig_combined".to_string()),
            redacted_content: None,
        }));

        // Should have signature_delta and content_block_stop
        assert!(
            events.iter().any(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "signature_delta"
            }),
            "should emit signature_delta"
        );
        assert!(
            events.iter().any(|e| e.event == "content_block_stop"),
            "should emit content_block_stop"
        );
        assert!(!ctx.in_thinking_block);
    }

    #[test]
    fn test_stop_reason_refusal_over_end_turn() {
        let mut mgr = SseStateManager::new();
        mgr.set_stop_reason("end_turn");
        assert_eq!(mgr.get_stop_reason(), "end_turn");
        mgr.set_stop_reason("refusal");
        assert_eq!(mgr.get_stop_reason(), "refusal");
    }

    #[test]
    fn test_stop_reason_pause_turn_over_end_turn() {
        let mut mgr = SseStateManager::new();
        mgr.set_stop_reason("end_turn");
        assert_eq!(mgr.get_stop_reason(), "end_turn");
        mgr.set_stop_reason("pause_turn");
        assert_eq!(mgr.get_stop_reason(), "pause_turn");
    }

    #[test]
    fn test_stop_reason_max_tokens_wins_over_all() {
        let mut mgr = SseStateManager::new();
        mgr.set_stop_reason("end_turn");
        mgr.set_stop_reason("pause_turn");
        mgr.set_stop_reason("refusal");
        mgr.set_stop_reason("max_tokens");
        assert_eq!(mgr.get_stop_reason(), "max_tokens");
    }

    #[test]
    fn test_message_start_includes_stop_sequence_null() {
        let ctx = StreamContext::new_with_thinking(
            "test-model",
            123,
            None,
            false,
            HashMap::new(),
            false,
            Vec::new(),
        );
        let msg_start = ctx.create_message_start_event();
        assert_eq!(
            msg_start["message"]["stop_sequence"],
            serde_json::Value::Null
        );
    }
}
