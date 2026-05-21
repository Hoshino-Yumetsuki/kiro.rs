//! 输入压缩管道
//!
//! 在协议转换完成后、发送到上游前，对 `ConversationState` 执行多层压缩，
//! 以规避 Kiro 上游请求体大小限制（实测约 5MiB 左右会触发 400）。
//!
//! 压缩顺序（低风险 → 高风险）：
//! 1. 空白压缩
//! 2. thinking 块丢弃/截断
//! 3. tool_result 智能截断
//! 4. tool_use input 截断
//! 5. 历史截断

use crate::kiro::model::requests::conversation::{ConversationState, Message};
use crate::model::config::CompressionConfig;

/// 压缩统计信息
#[derive(Debug, Default)]
pub struct CompressionStats {
    pub whitespace_saved: usize,
    pub thinking_saved: usize,
    pub tool_result_saved: usize,
    pub tool_use_input_saved: usize,
    pub history_turns_removed: usize,
    pub history_bytes_saved: usize,
}

impl CompressionStats {
    /// 总节省字节数
    pub fn total_saved(&self) -> usize {
        self.whitespace_saved
            + self.thinking_saved
            + self.tool_result_saved
            + self.tool_use_input_saved
            + self.history_bytes_saved
    }
}

/// 压缩管道入口
///
/// 按顺序执行各层压缩，返回统计信息。
pub fn compress(state: &mut ConversationState, config: &CompressionConfig) -> CompressionStats {
    let mut stats = CompressionStats::default();

    if !config.enabled {
        return stats;
    }

    // 1. 空白压缩
    if config.whitespace_compression {
        stats.whitespace_saved = compress_whitespace_pass(state);
    }

    // 2. thinking 丢弃/截断
    if config.thinking_strategy != "keep" {
        stats.thinking_saved = compress_thinking_pass(state, &config.thinking_strategy);
    }

    // 3. tool_result 智能截断
    if config.tool_result_max_chars > 0 {
        stats.tool_result_saved = compress_tool_results_pass(
            state,
            config.tool_result_max_chars,
            config.tool_result_head_lines,
            config.tool_result_tail_lines,
        );
    }

    // 4. tool_use input 截断
    if config.tool_use_input_max_chars > 0 {
        stats.tool_use_input_saved =
            compress_tool_use_inputs_pass(state, config.tool_use_input_max_chars);
    }

    // 5. 历史截断（最后手段）
    if config.max_history_turns > 0 || config.max_history_chars > 0 {
        let (turns, bytes) =
            compress_history_pass(state, config.max_history_turns, config.max_history_chars);
        stats.history_turns_removed = turns;
        stats.history_bytes_saved = bytes;
    }

    // 历史截断会破坏 tool_use(tool_uses) 与 tool_result(tool_results) 的跨消息配对：
    // assistant(tool_use) → user(tool_result)。
    // 若留下孤立 tool_use/tool_result，上游会返回 400 "Improperly formed request"。
    let (removed_tool_uses, removed_tool_results) = repair_tool_pairing_pass(state);
    if removed_tool_uses > 0 || removed_tool_results > 0 {
        tracing::debug!(
            removed_tool_uses,
            removed_tool_results,
            "压缩后已修复 tool_use/tool_result 配对"
        );
    }

    let repaired_non_empty_contents = repair_non_empty_content_pass(state);
    if repaired_non_empty_contents > 0 {
        tracing::debug!(repaired_non_empty_contents, "压缩后已修复空 content 占位符");
    }

    stats
}

// ============ 空白压缩 ============

/// 空白压缩：连续空行(3+)→单空行，行尾空格移除，保留行首缩进
fn compress_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut consecutive_empty = 0u32;

    for line in text.split('\n') {
        let trimmed_end = line.trim_end();

        if trimmed_end.is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 2 && !result.is_empty() {
                result.push('\n');
            }
        } else {
            consecutive_empty = 0;
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(trimmed_end);
        }
    }

    result
}

/// 对 ConversationState 中所有文本字段执行空白压缩
fn compress_whitespace_pass(state: &mut ConversationState) -> usize {
    let mut saved = 0usize;

    for msg in &mut state.history {
        match msg {
            Message::User(user_msg) => {
                saved += compress_string_field(&mut user_msg.user_input_message.content);
            }
            Message::Assistant(assistant_msg) => {
                saved +=
                    compress_string_field(&mut assistant_msg.assistant_response_message.content);
            }
        }
    }

    saved += compress_string_field(&mut state.current_message.user_input_message.content);
    saved
}

/// 压缩单个字符串字段，返回节省的字节数
///
/// 跳过仅为空格占位符 " " 的字段（Kiro API 要求 content 不能为空，
/// converter 使用 " " 作为占位符）
fn compress_string_field(field: &mut String) -> usize {
    if field == " " {
        return 0;
    }
    let original_len = field.len();
    let compressed = compress_whitespace(field);
    if compressed.len() < original_len {
        let saved = original_len - compressed.len();
        *field = compressed;
        saved
    } else {
        0
    }
}

// ============ thinking 压缩 ============

/// 处理 history 中 assistant 消息的 `<thinking>...</thinking>` 块
fn compress_thinking_pass(state: &mut ConversationState, strategy: &str) -> usize {
    let mut saved = 0usize;

    for msg in &mut state.history {
        if let Message::Assistant(assistant_msg) = msg {
            let content = &mut assistant_msg.assistant_response_message.content;
            let original_len = content.len();

            match strategy {
                "discard" => *content = remove_thinking_blocks(content),
                "truncate" => *content = truncate_thinking_blocks(content, 500),
                _ => {}
            }

            if content.len() < original_len {
                saved += original_len - content.len();
            }
        }
    }

    saved
}

/// 移除所有 `<thinking>...</thinking>` 块
fn remove_thinking_blocks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find("<thinking>") {
        result.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("</thinking>") {
            remaining = &remaining[start + end + "</thinking>".len()..];
        } else {
            remaining = "";
        }
    }
    result.push_str(remaining);
    result
}

/// 截断 `<thinking>...</thinking>` 块内容，保留前 N 个字符
fn truncate_thinking_blocks(text: &str, max_chars: usize) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find("<thinking>") {
        result.push_str(&remaining[..start]);
        let after_tag = &remaining[start + "<thinking>".len()..];

        if let Some(end) = after_tag.find("</thinking>") {
            let thinking_content = &after_tag[..end];
            let truncated = safe_char_truncate(thinking_content, max_chars);
            result.push_str("<thinking>");
            result.push_str(truncated);
            if truncated.len() < thinking_content.len() {
                result.push_str("...[truncated]");
            }
            result.push_str("</thinking>");
            remaining = &after_tag[end + "</thinking>".len()..];
        } else {
            let truncated = safe_char_truncate(after_tag, max_chars);
            result.push_str("<thinking>");
            result.push_str(truncated);
            result.push_str("...[truncated]</thinking>");
            remaining = "";
        }
    }
    result.push_str(remaining);
    result
}

// ============ tool_result 智能截断 ============

/// 按行智能截断，保留头尾行
fn smart_truncate_by_lines(
    text: &str,
    max_chars: usize,
    head_lines: usize,
    tail_lines: usize,
) -> (String, usize) {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return (text.to_string(), 0);
    }

    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    if total_lines <= head_lines + tail_lines {
        let half = max_chars / 2;
        let head = safe_char_truncate(text, half);
        let tail_chars = max_chars.saturating_sub(head.chars().count());
        let tail_start = text
            .char_indices()
            .rev()
            .nth(tail_chars.saturating_sub(1))
            .map(|(i, _)| i)
            .unwrap_or(0);
        let tail = &text[tail_start..];
        let omitted = char_count.saturating_sub(head.chars().count() + tail.chars().count());
        let result = format!("{}\n... [{} chars omitted] ...\n{}", head, omitted, tail);
        let saved = text.len().saturating_sub(result.len());
        return (result, saved);
    }

    let head_part: String = lines[..head_lines].join("\n");
    let tail_part: String = lines[total_lines - tail_lines..].join("\n");
    let omitted_lines = total_lines - head_lines - tail_lines;
    let omitted_chars =
        char_count.saturating_sub(head_part.chars().count() + tail_part.chars().count());

    let mut result = format!(
        "{}\n... [{} lines omitted ({} chars)] ...\n{}",
        head_part, omitted_lines, omitted_chars, tail_part
    );

    // 硬截断兜底：确保结果不超过 max_chars
    if result.chars().count() > max_chars {
        let truncated = safe_char_truncate(&result, max_chars);
        result = truncated.to_string();
    }

    let saved = text.len().saturating_sub(result.len());
    (result, saved)
}

/// 遍历所有 tool_result 的 text 字段，执行智能截断
fn compress_tool_results_pass(
    state: &mut ConversationState,
    max_chars: usize,
    head_lines: usize,
    tail_lines: usize,
) -> usize {
    let mut saved = 0usize;

    for msg in &mut state.history {
        if let Message::User(user_msg) = msg {
            for result in &mut user_msg
                .user_input_message
                .user_input_message_context
                .tool_results
            {
                saved += truncate_tool_result_content(
                    &mut result.content,
                    max_chars,
                    head_lines,
                    tail_lines,
                );
            }
        }
    }

    for result in &mut state
        .current_message
        .user_input_message
        .user_input_message_context
        .tool_results
    {
        saved +=
            truncate_tool_result_content(&mut result.content, max_chars, head_lines, tail_lines);
    }

    saved
}

/// 截断单个 tool_result 的 content 数组中的 text 字段
fn truncate_tool_result_content(
    content: &mut [serde_json::Map<String, serde_json::Value>],
    max_chars: usize,
    head_lines: usize,
    tail_lines: usize,
) -> usize {
    let mut saved = 0usize;

    for map in content.iter_mut() {
        if let Some(serde_json::Value::String(text)) = map.get_mut("text")
            && text.chars().count() > max_chars
        {
            let (truncated, s) = smart_truncate_by_lines(text, max_chars, head_lines, tail_lines);
            saved += s;
            *text = truncated;
        }
    }

    saved
}

// ============ tool_use input 截断 ============

/// 遍历 history 中 assistant 消息的 tool_use input，截断大字符串字段
fn compress_tool_use_inputs_pass(state: &mut ConversationState, max_chars: usize) -> usize {
    let mut saved = 0usize;

    for msg in &mut state.history {
        if let Message::Assistant(assistant_msg) = msg
            && let Some(ref mut tool_uses) = assistant_msg.assistant_response_message.tool_uses
        {
            for tool_use in tool_uses.iter_mut() {
                let serialized = serde_json::to_string(&tool_use.input).unwrap_or_default();
                if serialized.chars().count() > max_chars {
                    saved += truncate_json_value_strings(&mut tool_use.input, max_chars);
                }
            }
        }
    }

    saved
}

/// 递归截断 JSON 值中的大字符串
fn truncate_json_value_strings(value: &mut serde_json::Value, max_chars: usize) -> usize {
    let mut saved = 0usize;

    match value {
        serde_json::Value::String(s) => {
            let original_char_count = s.chars().count();
            if original_char_count > max_chars {
                let original_len = s.len();
                let truncated = safe_char_truncate(s, max_chars).to_string();
                let omitted_chars = original_char_count.saturating_sub(max_chars);

                // 仅当“带标记版本”确实更短时才附加标记，避免在边界场景（仅略超阈值）
                // 反而把字符串变长，导致压缩失效。
                let with_marker = format!(
                    "{}...[truncated {} chars]",
                    truncated.as_str(),
                    omitted_chars
                );
                let new_value = if with_marker.len() < original_len {
                    with_marker
                } else {
                    truncated
                };

                saved += original_len.saturating_sub(new_value.len());
                *s = new_value;
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                saved += truncate_json_value_strings(v, max_chars);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                saved += truncate_json_value_strings(v, max_chars);
            }
        }
        _ => {}
    }

    saved
}

// ============ 历史截断 ============

/// 历史截断：保留前 2 条（系统消息对），从前往后成对移除
///
/// 返回 (移除的轮数, 移除的字节数)
fn compress_history_pass(
    state: &mut ConversationState,
    max_turns: usize,
    max_chars: usize,
) -> (usize, usize) {
    let mut removed = 0usize;
    let mut bytes_saved = 0usize;
    let preserve_count = 2;

    /// 计算一条消息的字节数
    fn msg_bytes(msg: &Message) -> usize {
        match msg {
            Message::User(u) => u.user_input_message.content.len(),
            Message::Assistant(a) => a.assistant_response_message.content.len(),
        }
    }

    // 按轮数截断
    if max_turns > 0 {
        let max_messages = preserve_count + max_turns * 2;
        while state.history.len() > max_messages && state.history.len() > preserve_count + 2 {
            bytes_saved += msg_bytes(&state.history[preserve_count]);
            state.history.remove(preserve_count);
            bytes_saved += msg_bytes(&state.history[preserve_count]);
            state.history.remove(preserve_count);
            removed += 1;
        }
    }

    // 按字符数截断
    if max_chars > 0 {
        loop {
            let total_chars: usize = state
                .history
                .iter()
                .map(|msg| match msg {
                    Message::User(u) => u.user_input_message.content.chars().count(),
                    Message::Assistant(a) => a.assistant_response_message.content.chars().count(),
                })
                .sum();

            if total_chars <= max_chars || state.history.len() <= preserve_count + 2 {
                break;
            }

            bytes_saved += msg_bytes(&state.history[preserve_count]);
            state.history.remove(preserve_count);
            bytes_saved += msg_bytes(&state.history[preserve_count]);
            state.history.remove(preserve_count);
            removed += 1;
        }
    }

    (removed, bytes_saved)
}

/// 修复 tool_use/tool_result 配对（压缩后）。
///
/// 目标：
/// - 移除 history/current 中孤立的 tool_result（其 tool_use_id 在 history 的 tool_use 中不存在）
/// - 移除 history 中孤立的 tool_use（其 tool_use_id 在 history/current 的 tool_result 中不存在）
///
/// 返回 (移除的 tool_use 数, 移除的 tool_result 数)。
fn repair_tool_pairing_pass(state: &mut ConversationState) -> (usize, usize) {
    use std::collections::HashSet;

    // 1) 收集 history 内所有 tool_use_id（上游通常要求 tool_result 必须能在历史 tool_use 中找到）
    let mut tool_use_ids: HashSet<String> = HashSet::new();
    for msg in &state.history {
        if let Message::Assistant(a) = msg
            && let Some(ref tool_uses) = a.assistant_response_message.tool_uses
        {
            for tu in tool_uses {
                tool_use_ids.insert(tu.tool_use_id.clone());
            }
        }
    }

    // 2) 移除 history/current 中孤立 tool_result（没有对应 tool_use）
    let mut removed_tool_results = 0usize;

    for msg in &mut state.history {
        if let Message::User(u) = msg {
            let results = &mut u.user_input_message.user_input_message_context.tool_results;
            let before = results.len();
            results.retain(|tr| tool_use_ids.contains(&tr.tool_use_id));
            removed_tool_results += before - results.len();
        }
    }

    {
        let results = &mut state
            .current_message
            .user_input_message
            .user_input_message_context
            .tool_results;
        let before = results.len();
        results.retain(|tr| tool_use_ids.contains(&tr.tool_use_id));
        removed_tool_results += before - results.len();
    }

    // 3) 收集 history/current 内所有 tool_result 的 tool_use_id
    let mut tool_result_ids: HashSet<String> = HashSet::new();
    for msg in &state.history {
        if let Message::User(u) = msg {
            for tr in &u.user_input_message.user_input_message_context.tool_results {
                tool_result_ids.insert(tr.tool_use_id.clone());
            }
        }
    }
    for tr in &state
        .current_message
        .user_input_message
        .user_input_message_context
        .tool_results
    {
        tool_result_ids.insert(tr.tool_use_id.clone());
    }

    // 4) 移除 history 内孤立 tool_use（没有对应 tool_result）
    let mut removed_tool_uses = 0usize;
    for msg in &mut state.history {
        if let Message::Assistant(a) = msg
            && let Some(ref mut tool_uses) = a.assistant_response_message.tool_uses
        {
            let before = tool_uses.len();
            tool_uses.retain(|tu| tool_result_ids.contains(&tu.tool_use_id));
            removed_tool_uses += before - tool_uses.len();

            if tool_uses.is_empty() {
                a.assistant_response_message.tool_uses = None;
            }
        }
    }

    (removed_tool_uses, removed_tool_results)
}

/// 修复空 content 字段（压缩后最终兜底）。
///
/// 规则：仅在必要时替换为 "."，优先保留真实结构。
/// 处理范围：
/// - history user_input_message.content（仅当无 images/tool_results 时兜底）
/// - history assistant_response_message.content（仅当无 tool_uses 时兜底）
/// - current_message.user_input_message.content（最终必要兜底）
/// - history/current tool_result content 数组中空 text 项（优先删除，必要时兜底）
fn repair_non_empty_content_pass(state: &mut ConversationState) -> usize {
    let mut repaired = 0usize;

    for msg in &mut state.history {
        match msg {
            Message::User(user_msg) => {
                // 仅当既无图片也无 tool_results 时才兜底空 content
                let has_payload = !user_msg.user_input_message.images.is_empty()
                    || !user_msg
                        .user_input_message
                        .user_input_message_context
                        .tool_results
                        .is_empty();
                if !has_payload && repair_content_field(&mut user_msg.user_input_message.content) {
                    repaired += 1;
                }
                repaired += repair_tool_result_text_fields(
                    &mut user_msg
                        .user_input_message
                        .user_input_message_context
                        .tool_results,
                );
            }
            Message::Assistant(assistant_msg) => {
                // 仅当 assistant 消息没有 tool_uses 时才修复空 content
                let has_tool_uses = assistant_msg
                    .assistant_response_message
                    .tool_uses
                    .as_ref()
                    .is_some_and(|tools| !tools.is_empty());

                if !has_tool_uses
                    && repair_content_field(&mut assistant_msg.assistant_response_message.content)
                {
                    repaired += 1;
                }
            }
        }
    }

    // current_message 仅在没有任何非文本载荷时才做最终兜底
    let current_has_payload = !state.current_message.user_input_message.images.is_empty()
        || !state
            .current_message
            .user_input_message
            .user_input_message_context
            .tool_results
            .is_empty();
    if !current_has_payload
        && repair_content_field(&mut state.current_message.user_input_message.content)
    {
        repaired += 1;
    }
    repaired += repair_tool_result_text_fields(
        &mut state
            .current_message
            .user_input_message
            .user_input_message_context
            .tool_results,
    );

    repaired
}

/// 修复 tool_result content 数组中 text 字段为空字符串的条目。
/// 策略：优先删除空 text 项；若删除后 content 变空数组则兜底补 "."。
fn repair_tool_result_text_fields(
    results: &mut [crate::kiro::model::requests::tool::ToolResult],
) -> usize {
    let mut repaired = 0usize;
    for result in results.iter_mut() {
        // 先删除所有空 text 项
        let original_len = result.content.len();
        result.content.retain(|map| {
            if let Some(serde_json::Value::String(text)) = map.get("text") {
                !text.trim().is_empty()
            } else {
                true
            }
        });
        let removed = original_len - result.content.len();

        // 若删除后 content 变空，兜底补一个 "." text 项
        if result.content.is_empty() {
            let mut map = serde_json::Map::new();
            map.insert(
                "text".to_string(),
                serde_json::Value::String(".".to_string()),
            );
            result.content.push(map);
            repaired += 1;
        } else if removed > 0 {
            repaired += removed;
        }
    }
    repaired
}

fn repair_content_field(field: &mut String) -> bool {
    if field.trim().is_empty() {
        *field = ".".to_string();
        return true;
    }
    false
}

// ============ 超长消息内容截断 ============

/// 截断超长的用户消息内容（history user messages 和 current_message）
///
/// 这是最后手段的压缩层，仅在自适应二次压缩中使用。
/// 截断策略：保留头部内容，尾部截断并附加省略标记。
///
/// 返回节省的字节数。
#[allow(dead_code)]
pub fn compress_long_messages_pass(state: &mut ConversationState, max_chars: usize) -> usize {
    if max_chars == 0 {
        return 0;
    }

    let mut saved = 0usize;

    // 遍历 history 中所有 User 消息
    for msg in &mut state.history {
        if let Message::User(user_msg) = msg {
            saved += truncate_long_content(&mut user_msg.user_input_message.content, max_chars);
        }
    }

    // 处理 current_message
    saved += truncate_long_content(
        &mut state.current_message.user_input_message.content,
        max_chars,
    );

    saved
}

/// 截断单个 content 字段，返回节省的字节数
///
/// 跳过仅为空格占位符 " " 的字段（与 compress_string_field 一致）
fn truncate_long_content(field: &mut String, max_chars: usize) -> usize {
    if field == " " {
        return 0;
    }
    let char_count = field.chars().count();
    if char_count <= max_chars {
        return 0;
    }

    let original_len = field.len();
    let truncated = safe_char_truncate(field, max_chars);
    let omitted = char_count - max_chars;
    *field = format!(
        "{}\n...[content truncated, {} chars omitted]",
        truncated, omitted
    );
    original_len.saturating_sub(field.len())
}

// ============ 工具函数 ============

/// 安全 UTF-8 字符截断
fn safe_char_truncate(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => &text[..idx],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiro::model::requests::conversation::*;
    use crate::kiro::model::requests::tool::{ToolResult, ToolUseEntry};
    use crate::model::config::CompressionConfig;

    fn make_simple_state(history_content: Vec<(&str, &str)>, current: &str) -> ConversationState {
        let mut history = Vec::new();
        for (user, assistant) in history_content {
            history.push(Message::User(HistoryUserMessage::new(
                user,
                "claude-sonnet-4.5",
            )));
            history.push(Message::Assistant(HistoryAssistantMessage::new(assistant)));
        }
        ConversationState::new("test-conv")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                current,
                "claude-sonnet-4.5",
            )))
            .with_history(history)
    }

    #[test]
    fn test_compress_whitespace_consecutive_empty_lines() {
        let input = "line1\n\n\n\n\nline2";
        let result = compress_whitespace(input);
        // 5 个空行 → 保留最多 2 个（即 line1 + 2 个 \n + line2）
        assert_eq!(result, "line1\n\n\nline2");
    }

    #[test]
    fn test_compress_whitespace_trailing_spaces() {
        let input = "hello   \nworld  ";
        let result = compress_whitespace(input);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_compress_whitespace_preserves_indentation() {
        let input = "    indented\n        more indented";
        let result = compress_whitespace(input);
        assert_eq!(result, "    indented\n        more indented");
    }

    #[test]
    fn test_smart_truncate_short_content_unchanged() {
        let input = "short text";
        let (result, saved) = smart_truncate_by_lines(input, 100, 5, 3);
        assert_eq!(result, input);
        assert_eq!(saved, 0);
    }

    #[test]
    fn test_smart_truncate_preserves_head_tail() {
        let lines: Vec<String> = (0..200).map(|i| format!("line {}", i)).collect();
        let input = lines.join("\n");
        let (result, _saved) = smart_truncate_by_lines(&input, 100, 3, 2);
        assert!(result.starts_with("line 0\nline 1\nline 2\n"));
        assert!(result.ends_with("line 198\nline 199"));
        assert!(result.contains("lines omitted"));
    }

    #[test]
    fn test_safe_char_truncate_utf8() {
        let input = "你好世界abcd";
        let result = safe_char_truncate(input, 4);
        assert_eq!(result, "你好世界");
    }

    #[test]
    fn test_thinking_discard() {
        let mut state = make_simple_state(
            vec![(
                "hi",
                "<thinking>long thinking content here</thinking>\n\nactual response",
            )],
            "next",
        );
        let config = CompressionConfig {
            thinking_strategy: "discard".to_string(),
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert!(stats.thinking_saved > 0);
        // assistant content 不应包含 thinking 标签
        if let Message::Assistant(a) = &state.history[1] {
            assert!(!a.assistant_response_message.content.contains("<thinking>"));
            assert!(
                a.assistant_response_message
                    .content
                    .contains("actual response")
            );
        }
    }

    #[test]
    fn test_thinking_truncate() {
        let long_thinking = "a".repeat(1000);
        let content = format!("<thinking>{}</thinking>\n\nresponse", long_thinking);
        let mut state = make_simple_state(vec![("hi", &content)], "next");
        let config = CompressionConfig {
            thinking_strategy: "truncate".to_string(),
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert!(stats.thinking_saved > 0);
        if let Message::Assistant(a) = &state.history[1] {
            assert!(a.assistant_response_message.content.contains("<thinking>"));
            assert!(a.assistant_response_message.content.contains("[truncated]"));
        }
    }

    #[test]
    fn test_thinking_keep() {
        let content = "<thinking>keep me</thinking>\n\nresponse";
        let mut state = make_simple_state(vec![("hi", content)], "next");
        let config = CompressionConfig {
            thinking_strategy: "keep".to_string(),
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert_eq!(stats.thinking_saved, 0);
        if let Message::Assistant(a) = &state.history[1] {
            assert!(
                a.assistant_response_message
                    .content
                    .contains("<thinking>keep me</thinking>")
            );
        }
    }

    #[test]
    fn test_compress_non_empty_content_repairs_whitespace_history_user() {
        let mut state = make_simple_state(vec![("   \n\n", "assistant")], "current");
        let config = CompressionConfig {
            whitespace_compression: true,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        if let Message::User(u) = &state.history[0] {
            assert_eq!(u.user_input_message.content, ".");
        } else {
            panic!("expected User message");
        }
    }

    #[test]
    fn test_compress_non_empty_content_repairs_whitespace_current() {
        let mut state = make_simple_state(vec![("user", "assistant")], "   ");
        let config = CompressionConfig {
            whitespace_compression: true,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);
        assert_eq!(state.current_message.user_input_message.content, ".");
    }

    #[test]
    fn test_compress_non_empty_content_repairs_thinking_discard() {
        let mut state = make_simple_state(
            vec![("hi", "<thinking>only thinking</thinking>   ")],
            "next",
        );
        let config = CompressionConfig {
            thinking_strategy: "discard".to_string(),
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        if let Message::Assistant(a) = &state.history[1] {
            assert_eq!(a.assistant_response_message.content, ".");
        } else {
            panic!("expected Assistant message");
        }
    }

    #[test]
    fn test_compress_non_empty_content_repairs_after_tool_pairing() {
        let tool_use_id = "tooluse_orphan";

        let assistant_msg = Message::Assistant(HistoryAssistantMessage::new(" "));

        let user_with_tool_result = Message::User(HistoryUserMessage {
            user_input_message: UserMessage::new(" ", "claude-sonnet-4.5").with_context(
                UserInputMessageContext::new()
                    .with_tool_results(vec![ToolResult::success(tool_use_id, "ok")]),
            ),
        });

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                "next",
                "claude-sonnet-4.5",
            )))
            .with_history(vec![assistant_msg, user_with_tool_result]);

        let config = CompressionConfig {
            max_history_turns: 0,
            max_history_chars: 0,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        if let Message::Assistant(a) = &state.history[0] {
            assert_eq!(a.assistant_response_message.content, ".");
        } else {
            panic!("expected Assistant message");
        }

        if let Message::User(u) = &state.history[1] {
            assert_eq!(u.user_input_message.content, ".");
            assert!(
                u.user_input_message
                    .user_input_message_context
                    .tool_results
                    .is_empty()
            );
        } else {
            panic!("expected User message");
        }
    }

    #[test]
    fn test_compress_non_empty_content_keeps_normal_content() {
        let mut state = make_simple_state(vec![("user", "assistant")], "current");
        let config = CompressionConfig {
            whitespace_compression: true,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        if let Message::User(u) = &state.history[0] {
            assert_eq!(u.user_input_message.content, "user");
        }
        if let Message::Assistant(a) = &state.history[1] {
            assert_eq!(a.assistant_response_message.content, "assistant");
        }
        assert_eq!(state.current_message.user_input_message.content, "current");
    }

    #[test]
    fn test_compress_non_empty_content_placeholder_is_idempotent() {
        let mut state = make_simple_state(vec![(".", ".")], ".");
        let config = CompressionConfig {
            whitespace_compression: true,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        if let Message::User(u) = &state.history[0] {
            assert_eq!(u.user_input_message.content, ".");
        }
        if let Message::Assistant(a) = &state.history[1] {
            assert_eq!(a.assistant_response_message.content, ".");
        }
        assert_eq!(state.current_message.user_input_message.content, ".");
    }

    #[test]
    fn test_compress_non_empty_content_repairs_empty_history_assistant_after_whitespace() {
        let mut state = make_simple_state(vec![("user", "\n\n   \n")], "current");
        let config = CompressionConfig {
            whitespace_compression: true,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        if let Message::Assistant(a) = &state.history[1] {
            assert_eq!(a.assistant_response_message.content, ".");
        } else {
            panic!("expected Assistant message");
        }
    }

    #[test]
    fn test_tool_result_truncation() {
        let long_text = "x\n".repeat(500);
        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(
                UserInputMessage::new("msg", "claude-sonnet-4.5").with_context(
                    UserInputMessageContext::new()
                        .with_tool_results(vec![ToolResult::success("t1", &long_text)]),
                ),
            ))
            .with_history(Vec::new());

        let config = CompressionConfig {
            tool_result_max_chars: 100,
            tool_result_head_lines: 3,
            tool_result_tail_lines: 2,
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert!(stats.tool_result_saved > 0);
    }

    #[test]
    fn test_tool_use_input_truncation() {
        let long_input = serde_json::json!({
            "content": "a".repeat(10000)
        });
        let mut assistant_msg = AssistantMessage::new("using tool");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("t1", "write").with_input(long_input),
        ]);

        // tool_use 必须有对应的 tool_result（Kiro 要求严格配对），否则会被压缩后的修复逻辑移除。
        let current = UserInputMessage::new(" ", "claude-sonnet-4.5").with_context(
            UserInputMessageContext::new().with_tool_results(vec![ToolResult::success("t1", "ok")]),
        );
        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(current))
            .with_history(vec![
                Message::User(HistoryUserMessage::new("do it", "claude-sonnet-4.5")),
                Message::Assistant(HistoryAssistantMessage {
                    assistant_response_message: assistant_msg,
                }),
            ]);

        let config = CompressionConfig {
            tool_use_input_max_chars: 100,
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert!(stats.tool_use_input_saved > 0);
    }

    #[test]
    fn test_tool_use_input_truncation_does_not_expand_near_threshold() {
        let long_input = serde_json::json!({
            "content": "a".repeat(101)
        });
        let mut assistant_msg = AssistantMessage::new("using tool");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("t1", "write").with_input(long_input),
        ]);

        let current = UserInputMessage::new(" ", "claude-sonnet-4.5").with_context(
            UserInputMessageContext::new().with_tool_results(vec![ToolResult::success("t1", "ok")]),
        );
        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(current))
            .with_history(vec![
                Message::User(HistoryUserMessage::new("do it", "claude-sonnet-4.5")),
                Message::Assistant(HistoryAssistantMessage {
                    assistant_response_message: assistant_msg,
                }),
            ]);

        let config = CompressionConfig {
            tool_use_input_max_chars: 100,
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert!(stats.tool_use_input_saved > 0);

        if let Message::Assistant(a) = &state.history[1]
            && let Some(tool_uses) = &a.assistant_response_message.tool_uses
            && let Some(content) = tool_uses[0].input["content"].as_str()
        {
            // 101 字符略超阈值时，不应追加标记导致更长；应退化为纯截断
            let expected = "a".repeat(100);
            assert_eq!(content, expected.as_str());
        } else {
            panic!("tool_use input content should exist");
        }
    }

    #[test]
    fn test_tool_use_input_truncation_unicode_under_limit_is_unchanged() {
        let original = "你".repeat(60); // 60 chars, but 180 bytes
        let long_input = serde_json::json!({
            "content": original.clone()
        });
        let mut assistant_msg = AssistantMessage::new("using tool");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("t1", "write").with_input(long_input),
        ]);

        let current = UserInputMessage::new(" ", "claude-sonnet-4.5").with_context(
            UserInputMessageContext::new().with_tool_results(vec![ToolResult::success("t1", "ok")]),
        );
        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(current))
            .with_history(vec![
                Message::User(HistoryUserMessage::new("do it", "claude-sonnet-4.5")),
                Message::Assistant(HistoryAssistantMessage {
                    assistant_response_message: assistant_msg,
                }),
            ]);

        let config = CompressionConfig {
            tool_use_input_max_chars: 100,
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert_eq!(stats.tool_use_input_saved, 0);

        if let Message::Assistant(a) = &state.history[1]
            && let Some(tool_uses) = &a.assistant_response_message.tool_uses
            && let Some(content) = tool_uses[0].input["content"].as_str()
        {
            assert_eq!(content, original.as_str());
        } else {
            panic!("tool_use input content should exist");
        }
    }

    #[test]
    fn test_history_truncation_preserves_system_pair() {
        // 创建 system pair (2) + 5 轮对话 (10) = 12 条消息
        let mut history_content = vec![("system prompt", "I will follow these instructions.")];
        for _i in 0..5 {
            history_content.push(("user msg", "assistant msg"));
        }
        let mut state = make_simple_state(history_content, "current");

        let config = CompressionConfig {
            max_history_turns: 2,
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert!(stats.history_turns_removed > 0);
        // 应保留 system pair (2) + 2 轮 (4) = 6 条
        assert_eq!(state.history.len(), 6);
        // 第一对应该是 system pair
        if let Message::User(u) = &state.history[0] {
            assert!(u.user_input_message.content.contains("system prompt"));
        }
    }

    #[test]
    fn test_history_truncation_repairs_tool_pairing() {
        // 构造典型 tool_use → tool_result 跨消息链路：
        // assistant(tool_use) 紧跟 user(tool_result)。
        // 当按 user+assistant 成对从前往后截断时，容易删掉 tool_use 而保留 tool_result。
        let tool_use_id = "tooluse_1";

        let system_user = Message::User(HistoryUserMessage::new("system", "claude-sonnet-4.5"));
        let system_assistant = Message::Assistant(HistoryAssistantMessage::new(
            "I will follow these instructions.",
        ));

        let user1 = Message::User(HistoryUserMessage::new("do something", "claude-sonnet-4.5"));

        let tool_use =
            ToolUseEntry::new(tool_use_id, "Read").with_input(serde_json::json!({"path": "a.txt"}));
        let assistant1 = Message::Assistant(HistoryAssistantMessage {
            assistant_response_message: AssistantMessage::new(" ").with_tool_uses(vec![tool_use]),
        });

        let tool_result_ctx = UserInputMessageContext::new()
            .with_tool_results(vec![ToolResult::success(tool_use_id, "ok")]);
        let user2 = Message::User(HistoryUserMessage {
            user_input_message: UserMessage::new(" ", "claude-sonnet-4.5")
                .with_context(tool_result_ctx),
        });

        let assistant2 = Message::Assistant(HistoryAssistantMessage::new("done"));

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                "next",
                "claude-sonnet-4.5",
            )))
            .with_history(vec![
                system_user,
                system_assistant,
                user1,
                assistant1,
                user2,
                assistant2,
            ]);

        // 将历史限制到 1 轮（2+2=4 条），触发截断：会移除 user1+assistant1。
        // 若不修复，user2 中的 tool_result 会变成 orphan，导致上游 400。
        let config = CompressionConfig {
            max_history_turns: 1,
            max_history_chars: 0,
            ..Default::default()
        };

        let _stats = compress(&mut state, &config);

        // history 中不应存在 tool_result（因为对应 tool_use 已被截断移除）
        for msg in &state.history {
            if let Message::User(u) = msg {
                assert!(
                    u.user_input_message
                        .user_input_message_context
                        .tool_results
                        .is_empty(),
                    "history 中不应残留孤立 tool_result"
                );
            }
        }
    }

    #[test]
    fn test_compress_disabled_no_change() {
        let content = "line1\n\n\n\n\nline2   ";
        let mut state = make_simple_state(vec![("hi", content)], "next");
        let original_content = content.to_string();

        let config = CompressionConfig {
            enabled: false,
            ..Default::default()
        };
        let stats = compress(&mut state, &config);
        assert_eq!(stats.total_saved(), 0);
        assert_eq!(stats.history_turns_removed, 0);
        // content 应保持不变
        if let Message::Assistant(a) = &state.history[1] {
            assert_eq!(a.assistant_response_message.content, original_content);
        }
    }

    #[test]
    fn test_compress_long_messages_truncates_current_message() {
        let long_content = "a".repeat(20000);
        let mut state = make_simple_state(vec![], &long_content);
        let saved = compress_long_messages_pass(&mut state, 8192);
        assert!(saved > 0);
        let content = &state.current_message.user_input_message.content;
        assert!(content.len() < long_content.len());
        assert!(content.contains("[content truncated,"));
        assert!(content.contains("chars omitted]"));
        // 头部应保留
        assert!(content.starts_with("aaaa"));
    }

    #[test]
    fn test_compress_long_messages_truncates_history_user() {
        let long_content = "b".repeat(20000);
        let mut state = make_simple_state(vec![(&long_content, "short reply")], "current");
        let saved = compress_long_messages_pass(&mut state, 8192);
        assert!(saved > 0);
        if let Message::User(u) = &state.history[0] {
            assert!(u.user_input_message.content.len() < long_content.len());
            assert!(u.user_input_message.content.contains("[content truncated,"));
        } else {
            panic!("expected User message");
        }
    }

    #[test]
    fn test_compress_long_messages_short_unchanged() {
        let mut state = make_simple_state(vec![("short user", "short assistant")], "short current");
        let saved = compress_long_messages_pass(&mut state, 8192);
        assert_eq!(saved, 0);
        assert_eq!(
            state.current_message.user_input_message.content,
            "short current"
        );
        if let Message::User(u) = &state.history[0] {
            assert_eq!(u.user_input_message.content, "short user");
        }
    }

    #[test]
    fn test_compress_long_messages_skips_placeholder() {
        let mut state = make_simple_state(vec![], " ");
        let saved = compress_long_messages_pass(&mut state, 1);
        assert_eq!(saved, 0);
        assert_eq!(state.current_message.user_input_message.content, " ");
    }

    #[test]
    fn test_compress_long_messages_zero_max_chars_noop() {
        let long_content = "x".repeat(20000);
        let mut state = make_simple_state(vec![], &long_content);
        let saved = compress_long_messages_pass(&mut state, 0);
        assert_eq!(saved, 0);
        assert_eq!(
            state.current_message.user_input_message.content,
            long_content
        );
    }

    #[test]
    fn test_repair_tool_result_empty_text_in_history() {
        // tool_result content[*].text 为空字符串时，若删除后无内容，应兜底为 "."
        let mut map = serde_json::Map::new();
        map.insert(
            "text".to_string(),
            serde_json::Value::String("".to_string()),
        );

        let tool_result = ToolResult {
            tool_use_id: "t1".to_string(),
            content: vec![map],
            status: Some("success".to_string()),
            is_error: false,
        };

        // 需要对应的 tool_use，否则会被 repair_tool_pairing_pass 移除
        let tool_use = ToolUseEntry::new("t1", "read");
        let assistant_msg = Message::Assistant(HistoryAssistantMessage {
            assistant_response_message: AssistantMessage::new("ok").with_tool_uses(vec![tool_use]),
        });

        let user_msg = Message::User(HistoryUserMessage {
            user_input_message: UserMessage::new("do it", "claude-sonnet-4.5")
                .with_context(UserInputMessageContext::new().with_tool_results(vec![tool_result])),
        });

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                "next",
                "claude-sonnet-4.5",
            )))
            .with_history(vec![assistant_msg, user_msg]);

        let config = CompressionConfig::default();
        let _stats = compress(&mut state, &config);

        if let Message::User(u) = &state.history[1] {
            let text =
                u.user_input_message.user_input_message_context.tool_results[0].content[0]["text"]
                    .as_str()
                    .unwrap();
            assert_eq!(text, ".", "删除空 text 后无内容时应兜底为 '.'");
        } else {
            panic!("expected User message");
        }
    }

    #[test]
    fn test_repair_tool_result_empty_text_in_current() {
        // current_message tool_result content[*].text 为空字符串时，若删除后无内容，应兜底为 "."
        let mut map = serde_json::Map::new();
        map.insert(
            "text".to_string(),
            serde_json::Value::String("".to_string()),
        );

        let tool_result = ToolResult {
            tool_use_id: "t2".to_string(),
            content: vec![map],
            status: Some("success".to_string()),
            is_error: false,
        };

        let tool_use = ToolUseEntry::new("t2", "write");
        let assistant_msg = Message::Assistant(HistoryAssistantMessage {
            assistant_response_message: AssistantMessage::new("ok").with_tool_uses(vec![tool_use]),
        });

        let current = UserInputMessage::new("result here", "claude-sonnet-4.5")
            .with_context(UserInputMessageContext::new().with_tool_results(vec![tool_result]));

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(current))
            .with_history(vec![assistant_msg]);

        let config = CompressionConfig::default();
        let _stats = compress(&mut state, &config);

        let text = state
            .current_message
            .user_input_message
            .user_input_message_context
            .tool_results[0]
            .content[0]["text"]
            .as_str()
            .unwrap();
        assert_eq!(text, ".", "删除空 text 后无内容时应兜底为 '.'");
    }

    #[test]
    fn test_repair_tool_result_whitespace_only_text() {
        // text 为纯空白（如 "  \n  "）时也应被修复
        let mut map = serde_json::Map::new();
        map.insert(
            "text".to_string(),
            serde_json::Value::String("  \n  ".to_string()),
        );

        let tool_result = ToolResult {
            tool_use_id: "t3".to_string(),
            content: vec![map],
            status: Some("success".to_string()),
            is_error: false,
        };

        let tool_use = ToolUseEntry::new("t3", "search");
        let assistant_msg = Message::Assistant(HistoryAssistantMessage {
            assistant_response_message: AssistantMessage::new("searching")
                .with_tool_uses(vec![tool_use]),
        });

        let user_msg = Message::User(HistoryUserMessage {
            user_input_message: UserMessage::new("go", "claude-sonnet-4.5")
                .with_context(UserInputMessageContext::new().with_tool_results(vec![tool_result])),
        });

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                "next",
                "claude-sonnet-4.5",
            )))
            .with_history(vec![assistant_msg, user_msg]);

        let config = CompressionConfig::default();
        let _stats = compress(&mut state, &config);

        if let Message::User(u) = &state.history[1] {
            let text =
                u.user_input_message.user_input_message_context.tool_results[0].content[0]["text"]
                    .as_str()
                    .unwrap();
            assert_eq!(text, ".", "纯空白 text 应被修复为 '.'");
        } else {
            panic!("expected User message");
        }
    }

    #[test]
    fn test_repair_tool_result_mixed_empty_and_nonempty_text_keeps_nonempty_only() {
        let mut empty_map = serde_json::Map::new();
        empty_map.insert(
            "text".to_string(),
            serde_json::Value::String("   ".to_string()),
        );

        let mut nonempty_map = serde_json::Map::new();
        nonempty_map.insert(
            "text".to_string(),
            serde_json::Value::String("hello".to_string()),
        );

        let tool_result = ToolResult {
            tool_use_id: "t5".to_string(),
            content: vec![empty_map, nonempty_map],
            status: Some("success".to_string()),
            is_error: false,
        };

        let tool_use = ToolUseEntry::new("t5", "echo");
        let assistant_msg = Message::Assistant(HistoryAssistantMessage {
            assistant_response_message: AssistantMessage::new("ok").with_tool_uses(vec![tool_use]),
        });

        let current = UserInputMessage::new("result here", "claude-sonnet-4.5")
            .with_context(UserInputMessageContext::new().with_tool_results(vec![tool_result]));

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(current))
            .with_history(vec![assistant_msg]);

        let config = CompressionConfig::default();
        let _stats = compress(&mut state, &config);

        let content = &state
            .current_message
            .user_input_message
            .user_input_message_context
            .tool_results[0]
            .content;
        assert_eq!(content.len(), 1, "应删除空 text 项，仅保留非空项");
        assert_eq!(content[0]["text"].as_str().unwrap(), "hello");
    }

    #[test]
    fn test_repair_tool_result_nonempty_text_unchanged() {
        // text 有实际内容时不应被修改
        let mut map = serde_json::Map::new();
        map.insert(
            "text".to_string(),
            serde_json::Value::String("hello".to_string()),
        );

        let tool_result = ToolResult {
            tool_use_id: "t4".to_string(),
            content: vec![map],
            status: Some("success".to_string()),
            is_error: false,
        };

        let tool_use = ToolUseEntry::new("t4", "echo");
        let assistant_msg = Message::Assistant(HistoryAssistantMessage {
            assistant_response_message: AssistantMessage::new("echoing")
                .with_tool_uses(vec![tool_use]),
        });

        let user_msg = Message::User(HistoryUserMessage {
            user_input_message: UserMessage::new("go", "claude-sonnet-4.5")
                .with_context(UserInputMessageContext::new().with_tool_results(vec![tool_result])),
        });

        let mut state = ConversationState::new("test")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                "next",
                "claude-sonnet-4.5",
            )))
            .with_history(vec![assistant_msg, user_msg]);

        let config = CompressionConfig::default();
        let _stats = compress(&mut state, &config);

        if let Message::User(u) = &state.history[1] {
            let text =
                u.user_input_message.user_input_message_context.tool_results[0].content[0]["text"]
                    .as_str()
                    .unwrap();
            assert_eq!(text, "hello", "非空 text 不应被修改");
        } else {
            panic!("expected User message");
        }
    }
}
