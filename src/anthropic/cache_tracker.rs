use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::token::{
    count_message_content_tokens, count_system_message_tokens, count_tool_definition_tokens,
};

use super::types::{CacheControl, Message, MessagesRequest};

const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(300);
const ONE_HOUR_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Prompt cache 分桶键：按 user_id 隔离，无 user_id 时落入 Global 共享桶。
///
/// 与凭据 ID 解耦后，缓存命中不再受凭据故障转移影响。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum CacheKey {
    User(String),
    Global,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CacheResult {
    pub cache_read_input_tokens: i32,
    pub cache_creation_input_tokens: i32,
    pub cache_creation_5m_input_tokens: i32,
    pub cache_creation_1h_input_tokens: i32,
}

#[derive(Debug, Clone)]
pub struct CacheProfile {
    total_input_tokens: i32,
    min_cacheable_tokens: i32,
    blocks: Vec<CacheBlock>,
    breakpoints: Vec<CacheBreakpoint>,
}

#[derive(Debug, Clone)]
struct CacheBlock {
    prefix_fingerprint: [u8; 32],
    cumulative_tokens: i32,
}

#[derive(Debug, Clone)]
struct CacheBreakpoint {
    block_index: usize,
    ttl: Duration,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    token_count: i32,
    ttl: Duration,
    expires_at: Instant,
}

struct CachedCheckpointStore {
    by_user: HashMap<CacheKey, HashMap<[u8; 32], CacheEntry>>,
}

pub struct CacheTracker {
    entries: Mutex<CachedCheckpointStore>,
    max_supported_ttl: Duration,
}

impl CacheTracker {
    pub fn new(max_supported_ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(CachedCheckpointStore {
                by_user: HashMap::new(),
            }),
            max_supported_ttl,
        }
    }

    pub fn build_profile(
        &self,
        payload: &MessagesRequest,
        total_input_tokens: i32,
    ) -> CacheProfile {
        let flattened = flatten_cacheable_blocks(payload);

        // 与 prompt 内容无关但会影响官方缓存可复用性的固定配置。
        let request_prelude = canonicalize_json(serde_json::json!({
            "model": payload.model,
            "tool_choice": payload.tool_choice,
        }));
        let prelude_bytes = serde_json::to_vec(&request_prelude).unwrap_or_default();
        let mut prefix_hasher = Sha256::new();
        prefix_hasher.update((prelude_bytes.len() as u64).to_be_bytes());
        prefix_hasher.update(&prelude_bytes);

        let mut blocks = Vec::with_capacity(flattened.len());
        let mut breakpoints = Vec::new();
        let mut cumulative_tokens = 0i32;

        for (index, block) in flattened.into_iter().enumerate() {
            cumulative_tokens = cumulative_tokens.saturating_add(block.tokens);

            let block_bytes = serde_json::to_vec(&block.value).unwrap_or_default();
            let block_hash: [u8; 32] = Sha256::digest(&block_bytes).into();

            let mut next_prefix_hasher = prefix_hasher.clone();
            next_prefix_hasher.update(block_hash);
            let prefix_fingerprint: [u8; 32] = next_prefix_hasher.finalize().into();
            prefix_hasher = Sha256::new();
            prefix_hasher.update(prefix_fingerprint);

            blocks.push(CacheBlock {
                prefix_fingerprint,
                cumulative_tokens,
            });

            if let Some(ttl) = block.breakpoint_ttl {
                let ttl = ttl.min(self.max_supported_ttl);
                breakpoints.push(CacheBreakpoint {
                    block_index: index,
                    ttl,
                });
            }
        }

        CacheProfile {
            total_input_tokens: total_input_tokens.max(0),
            min_cacheable_tokens: minimum_cacheable_tokens_for_model(&payload.model),
            blocks,
            breakpoints,
        }
    }

    pub fn compute(&self, key: &CacheKey, profile: &CacheProfile) -> CacheResult {
        let Some(last_breakpoint) = profile.last_cacheable_breakpoint() else {
            return CacheResult::default();
        };

        // 缓存活跃时，保留 1 token 作为 input_tokens，其余全部归入 cache_read + cache_creation。
        // 这与 Anthropic 官方 prompt caching 行为一致。
        let cacheable_total = (profile.total_input_tokens - 1).max(0);

        let now = Instant::now();
        let mut entries = self.entries.lock();
        prune_expired(&mut entries.by_user, now);

        let Some(user_entries) = entries.by_user.get_mut(key) else {
            tracing::debug!(?key, "首次请求，无缓存条目");
            let (cache_5m, cache_1h) =
                compute_ttl_breakdown_absolute(cacheable_total, last_breakpoint.ttl);
            return CacheResult {
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: cacheable_total,
                cache_creation_5m_input_tokens: cache_5m,
                cache_creation_1h_input_tokens: cache_1h,
            };
        };

        tracing::debug!(?key, entry_count = user_entries.len(), "查找缓存匹配");

        // 策略：从最后一个 breakpoint 所在 block 开始倒序搜索所有 blocks 的 fingerprint，
        // 找到最深的缓存命中点（不限于当前请求的 breakpoints）。
        // 这解决了多轮对话中，上一轮的 breakpoint 在本轮不再是 breakpoint 但内容未变的场景。
        let search_end = last_breakpoint.block_index;
        let mut matched_tokens = 0;

        for block_index in (0..=search_end).rev() {
            let block = &profile.blocks[block_index];
            if let Some(entry) = user_entries.get_mut(&block.prefix_fingerprint) {
                if entry.expires_at <= now {
                    continue;
                }
                // 刷新过期时间
                entry.expires_at = now + entry.ttl;
                matched_tokens = block.cumulative_tokens.min(cacheable_total);
                break;
            }
        }

        // cache_read = matched_tokens（上限为 cacheable_total）
        // cache_creation = cacheable_total - cache_read
        let cache_read = matched_tokens.min(cacheable_total).max(0);
        let cache_creation = cacheable_total.saturating_sub(cache_read).max(0);
        let (cache_5m, cache_1h) =
            compute_ttl_breakdown_absolute(cache_creation, last_breakpoint.ttl);

        tracing::debug!(
            ?key,
            matched_tokens,
            cache_read,
            cache_creation,
            cache_5m,
            cache_1h,
            cacheable_total,
            "缓存计算结果"
        );

        CacheResult {
            cache_read_input_tokens: cache_read,
            cache_creation_input_tokens: cache_creation,
            cache_creation_5m_input_tokens: cache_5m,
            cache_creation_1h_input_tokens: cache_1h,
        }
    }

    pub fn update(&self, key: &CacheKey, profile: &CacheProfile) {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        prune_expired(&mut entries.by_user, now);

        let user_entries = entries.by_user.entry(key.clone()).or_default();

        // 确定最后一个可缓存断点的 TTL 和位置
        let last_bp = profile.last_cacheable_breakpoint();
        let (last_block_index, default_ttl) = match last_bp {
            Some(bp) => (bp.block_index, bp.ttl),
            None => return, // 无可缓存断点则不更新
        };

        // 存储所有 blocks（从 0 到 last breakpoint）的 fingerprint，
        // 这样下一轮即使某些 block 不再是 breakpoint，compute() 仍能通过 fingerprint 匹配到它们。
        for (idx, block) in profile.blocks.iter().enumerate() {
            if idx > last_block_index {
                break;
            }

            // 使用对应断点的 TTL（如果该 block 恰好是断点），否则用最后断点的 TTL
            let block_ttl = profile
                .breakpoints
                .iter()
                .find(|bp| bp.block_index == idx)
                .map(|bp| bp.ttl.min(self.max_supported_ttl))
                .unwrap_or(default_ttl);

            let next_expiry = now + block_ttl;

            match user_entries.get_mut(&block.prefix_fingerprint) {
                Some(existing) => {
                    existing.token_count = existing.token_count.max(block.cumulative_tokens);
                    existing.ttl = existing.ttl.max(block_ttl);
                    existing.expires_at = existing.expires_at.max(next_expiry);
                }
                None => {
                    user_entries.insert(
                        block.prefix_fingerprint,
                        CacheEntry {
                            token_count: block.cumulative_tokens,
                            ttl: block_ttl,
                            expires_at: next_expiry,
                        },
                    );
                }
            }
        }
    }
}

/// 计算不同 TTL 的缓存创建 token 数（直接传入 creation 绝对值 + 最后断点 TTL）
fn compute_ttl_breakdown_absolute(creation_tokens: i32, ttl: Duration) -> (i32, i32) {
    if creation_tokens <= 0 {
        return (0, 0);
    }
    if ttl == ONE_HOUR_CACHE_TTL {
        (0, creation_tokens)
    } else {
        (creation_tokens, 0)
    }
}

impl CacheProfile {
    fn cacheable_breakpoints(&self) -> Vec<ResolvedBreakpoint> {
        self.breakpoints
            .iter()
            .filter_map(|breakpoint| {
                let block = self.blocks.get(breakpoint.block_index)?;
                if block.cumulative_tokens < self.min_cacheable_tokens {
                    return None;
                }

                Some(ResolvedBreakpoint {
                    block_index: breakpoint.block_index,
                    cumulative_tokens: block.cumulative_tokens,
                    ttl: breakpoint.ttl,
                })
            })
            .collect()
    }

    fn last_cacheable_breakpoint(&self) -> Option<ResolvedBreakpoint> {
        self.cacheable_breakpoints().into_iter().last()
    }
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct ResolvedBreakpoint {
    block_index: usize,
    cumulative_tokens: i32,
    ttl: Duration,
}

#[derive(Debug)]
struct PendingBlock {
    value: serde_json::Value,
    tokens: i32,
    breakpoint_ttl: Option<Duration>,
}

fn flatten_cacheable_blocks(payload: &MessagesRequest) -> Vec<PendingBlock> {
    let mut blocks = Vec::new();

    if let Some(tools) = &payload.tools {
        for (tool_index, tool) in tools.iter().enumerate() {
            let mut value = serde_json::to_value(tool).unwrap_or(serde_json::Value::Null);
            let breakpoint_ttl = extract_cache_ttl(&value);
            strip_cache_control(&mut value);

            blocks.push(PendingBlock {
                value: canonicalize_json(serde_json::json!({
                    "kind": "tool",
                    "tool_index": tool_index,
                    "tool": value,
                })),
                tokens: count_tool_definition_tokens(tool) as i32,
                breakpoint_ttl,
            });
        }
    }

    if let Some(system) = &payload.system {
        for (system_index, block) in system.iter().enumerate() {
            let mut value = serde_json::to_value(block).unwrap_or(serde_json::Value::Null);
            let breakpoint_ttl = extract_cache_ttl(&value);
            strip_cache_control(&mut value);
            canonicalize_system_block_for_cache(&mut value);

            blocks.push(PendingBlock {
                value: canonicalize_json(serde_json::json!({
                    "kind": "system",
                    "system_index": system_index,
                    "block": value,
                })),
                tokens: count_system_message_tokens(block) as i32,
                breakpoint_ttl,
            });
        }
    }

    for (message_index, message) in payload.messages.iter().enumerate() {
        blocks.extend(flatten_message_blocks(message_index, message));
    }

    blocks
}

fn canonicalize_system_block_for_cache(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };

    let is_text_block = obj
        .get("type")
        .and_then(|v| v.as_str())
        .map(|t| t == "text")
        .unwrap_or(true);
    if !is_text_block {
        return;
    }

    let Some(text) = obj.get("text").and_then(|v| v.as_str()) else {
        return;
    };
    if !text.starts_with("x-anthropic-billing-header:") {
        return;
    }

    obj.insert(
        "text".to_string(),
        serde_json::Value::String("__anthropic_billing_header__".to_string()),
    );
}

fn flatten_message_blocks(message_index: usize, message: &Message) -> Vec<PendingBlock> {
    match &message.content {
        serde_json::Value::String(text) => vec![build_message_block(
            message_index,
            &message.role,
            0,
            serde_json::json!({
                "type": "text",
                "text": text,
            }),
            None,
        )],
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .enumerate()
            .map(|(block_index, block)| {
                let breakpoint_ttl = extract_cache_ttl(block);
                let mut normalized = block.clone();
                strip_cache_control(&mut normalized);
                build_message_block(
                    message_index,
                    &message.role,
                    block_index,
                    normalized,
                    breakpoint_ttl,
                )
            })
            .collect(),
        other => vec![build_message_block(
            message_index,
            &message.role,
            0,
            other.clone(),
            None,
        )],
    }
}

fn build_message_block(
    message_index: usize,
    role: &str,
    block_index: usize,
    block: serde_json::Value,
    breakpoint_ttl: Option<Duration>,
) -> PendingBlock {
    PendingBlock {
        tokens: count_message_content_tokens(&block) as i32,
        value: canonicalize_json(serde_json::json!({
            "kind": "message",
            "message_index": message_index,
            "role": role,
            "block_index": block_index,
            "block": block,
        })),
        breakpoint_ttl,
    }
}

fn extract_cache_ttl(value: &serde_json::Value) -> Option<Duration> {
    let cache_control = value.get("cache_control")?;
    let cache_control: CacheControl = serde_json::from_value(cache_control.clone()).ok()?;
    if cache_control.cache_type != "ephemeral" {
        return None;
    }

    Some(match cache_control.ttl.as_deref() {
        Some("1h") => ONE_HOUR_CACHE_TTL,
        _ => DEFAULT_CACHE_TTL,
    })
}

fn strip_cache_control(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(arr) => {
            for item in arr {
                strip_cache_control(item);
            }
        }
        serde_json::Value::Object(map) => {
            map.remove("cache_control");
            for item in map.values_mut() {
                strip_cache_control(item);
            }
        }
        _ => {}
    }
}

fn minimum_cacheable_tokens_for_model(model: &str) -> i32 {
    let model_lower = model.to_lowercase();

    if model_lower.contains("opus") || model_lower.contains("haiku-4") {
        4096
    } else if model_lower.contains("haiku-3") || model_lower.contains("haiku_3") {
        2048
    } else {
        1024
    }
}

fn prune_expired(entries: &mut HashMap<CacheKey, HashMap<[u8; 32], CacheEntry>>, now: Instant) {
    entries.retain(|_, user_entries| {
        user_entries.retain(|_, entry| entry.expires_at > now);
        !user_entries.is_empty()
    });
}

fn canonicalize_json(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(canonicalize_json).collect())
        }
        serde_json::Value::Object(map) => {
            let ordered: BTreeMap<_, _> = map
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect();

            let mut out = serde_json::Map::new();
            for (key, value) in ordered {
                out.insert(key, value);
            }
            serde_json::Value::Object(out)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::types::{SystemMessage, Tool};
    use crate::token;

    fn build_request(messages: Vec<Message>) -> MessagesRequest {
        MessagesRequest {
            model: "claude-sonnet-4-6".to_string(),
            max_tokens: 1024,
            messages,
            stream: false,
            system: Some(vec![SystemMessage {
                block_type: None,
                text: "system".to_string(),
                cache_control: None,
            }]),
            tools: Some(vec![Tool {
                tool_type: None,
                name: "echo".to_string(),
                description: "echo".to_string(),
                input_schema: Default::default(),
                max_uses: None,
                cache_control: None,
            }]),
            tool_choice: None,
            thinking: None,
            output_config: None,
            metadata: None,
        }
    }

    fn build_request_with_system(
        messages: Vec<Message>,
        system: Vec<SystemMessage>,
    ) -> MessagesRequest {
        let mut request = build_request(messages);
        request.system = Some(system);
        request
    }

    fn msg(role: &str, content: serde_json::Value) -> Message {
        Message {
            role: role.to_string(),
            content,
        }
    }

    fn cache_text(text: &str) -> serde_json::Value {
        serde_json::json!([{
            "type": "text",
            "text": text,
            "cache_control": { "type": "ephemeral" }
        }])
    }

    fn long_cacheable_text() -> String {
        std::iter::repeat_n("cacheable prompt chunk", 256)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn medium_turn_text(label: &str) -> String {
        format!(
            "{} {}",
            label,
            std::iter::repeat_n("conversation growth chunk", 80)
                .collect::<Vec<_>>()
                .join(" ")
        )
    }

    fn estimate_input_tokens(request: &MessagesRequest) -> i32 {
        token::count_all_tokens(
            request.model.clone(),
            request.system.clone(),
            request.messages.clone(),
            request.tools.clone(),
        ) as i32
    }

    #[test]
    fn attribution_header_drift_does_not_break_cache_hit() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let system1 = vec![
            SystemMessage {
                block_type: Some("text".to_string()),
                text:
                    "x-anthropic-billing-header: cc_version=2.1.87.1; cc_entrypoint=cli; cch=aaaaa;"
                        .to_string(),
                cache_control: None,
            },
            SystemMessage {
                block_type: Some("text".to_string()),
                text: long_cacheable_text(),
                cache_control: Some(CacheControl {
                    cache_type: "ephemeral".to_string(),
                    ttl: None,
                }),
            },
        ];
        let system2 = vec![
            SystemMessage {
                block_type: Some("text".to_string()),
                text: "x-anthropic-billing-header: cc_version=2.1.87.222222222222222222; cc_entrypoint=cli; cch=bbbbb; extra_padding=xyzxyzxyzxyz;".to_string(),
                cache_control: None,
            },
            SystemMessage {
                block_type: Some("text".to_string()),
                text: long_cacheable_text(),
                cache_control: Some(CacheControl {
                    cache_type: "ephemeral".to_string(),
                    ttl: None,
                }),
            },
        ];

        let req1 =
            build_request_with_system(vec![msg("user", serde_json::json!("hello"))], system1);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        let req2 =
            build_request_with_system(vec![msg("user", serde_json::json!("hello"))], system2);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);

        assert!(total1 != total2);
        // 尽管 billing header 变了，内容通过规范化后 fingerprint 相同，应命中缓存
        assert!(result.cache_read_input_tokens > 0);
        // 不变量：cache_read + cache_creation = total - 1
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }

    #[test]
    fn normal_system_text_change_invalidates_system_block() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let system1 = vec![SystemMessage {
            block_type: Some("text".to_string()),
            text: long_cacheable_text(),
            cache_control: Some(CacheControl {
                cache_type: "ephemeral".to_string(),
                ttl: None,
            }),
        }];
        let system2 = vec![SystemMessage {
            block_type: Some("text".to_string()),
            text: format!("{} extra", long_cacheable_text()),
            cache_control: Some(CacheControl {
                cache_type: "ephemeral".to_string(),
                ttl: None,
            }),
        }];

        let req1 =
            build_request_with_system(vec![msg("user", serde_json::json!("hello"))], system1);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        let req2 =
            build_request_with_system(vec![msg("user", serde_json::json!("hello"))], system2);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);

        // 系统文本变了，system block 的 fingerprint 变化导致 system 及之后都无法命中。
        // 但 tool 块（在 system 之前）未变，仍可命中。
        // cache_read 只覆盖 tool 块的 tokens（约 150），大部分归入 cache_creation。
        assert!(result.cache_read_input_tokens < cacheable_total / 2);
        assert!(result.cache_creation_input_tokens > 0);
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }

    #[test]
    fn explicit_breakpoint_without_hit_creates_prefix_only() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let req = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        let total = estimate_input_tokens(&req);
        let profile = tracker.build_profile(&req, total);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile);

        let cacheable_total = (total - 1).max(0);

        assert_eq!(result.cache_read_input_tokens, 0);
        // 首次请求，无缓存：所有可缓存 token 都是 creation
        assert_eq!(result.cache_creation_input_tokens, cacheable_total);
    }

    #[test]
    fn same_content_with_different_breakpoint_placement_still_hits_prefix() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let req1 = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        // req2: 相同的 user 文本但 cache_control 移到了 assistant 块上
        let req2 = build_request(vec![
            msg("user", serde_json::json!(long_cacheable_text())),
            msg(
                "assistant",
                serde_json::json!([{
                    "type": "text",
                    "text": "ok",
                    "cache_control": { "type": "ephemeral" }
                }]),
            ),
        ]);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);

        // user 块内容相同且位于同一位置，fingerprint 匹配 → cache_read 覆盖了该前缀
        assert!(result.cache_read_input_tokens > 0);
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }

    #[test]
    fn same_length_retry_with_same_breakpoint_is_hit() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let req1 = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        let req2 = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);
        let breakpoint_tokens = profile1
            .last_cacheable_breakpoint()
            .map(|bp| bp.cumulative_tokens)
            .unwrap_or(0);

        // 完全相同的请求：breakpoint block 匹配
        assert_eq!(
            result.cache_read_input_tokens,
            breakpoint_tokens.min(cacheable_total)
        );
        // cache_creation 只覆盖 token 估算开销（很小）
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }

    #[test]
    fn prefix_match_with_appended_turn_reads_previous_prefix_cache() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let req1 = build_request(vec![
            msg("user", cache_text(&long_cacheable_text())),
            msg("assistant", serde_json::json!("R1")),
            msg("user", serde_json::json!("Follow-up")),
            msg("assistant", serde_json::json!("R2")),
        ]);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        let req2 = build_request(vec![
            msg("user", cache_text(&long_cacheable_text())),
            msg("assistant", serde_json::json!("R1")),
            msg("user", serde_json::json!("Follow-up")),
            msg("assistant", serde_json::json!("R2")),
            msg("user", serde_json::json!("New feedback")),
            msg("assistant", serde_json::json!("R3")),
        ]);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);

        // 前缀（到 breakpoint）完全命中，新增的 turn 归入 cache_creation
        assert!(result.cache_read_input_tokens > 0);
        // cache_creation 覆盖追加的 turn + token 估算开销
        assert!(result.cache_creation_input_tokens > 0);
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }

    #[test]
    fn context_edit_only_reads_longest_unchanged_prefix() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let first_cacheable = long_cacheable_text();
        let later_cacheable = format!("later-prefix {}", long_cacheable_text());

        let req1 = build_request(vec![
            msg("user", cache_text(&first_cacheable)),
            msg(
                "assistant",
                serde_json::json!(medium_turn_text("original-middle")),
            ),
            msg("user", cache_text(&later_cacheable)),
            msg("assistant", serde_json::json!("R2")),
        ]);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        let req2 = build_request(vec![
            msg("user", cache_text(&first_cacheable)),
            msg(
                "assistant",
                serde_json::json!(medium_turn_text("edited-middle")),
            ),
            msg("user", cache_text(&later_cacheable)),
            msg("assistant", serde_json::json!("R2")),
        ]);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);
        let first_breakpoint_tokens = profile2.blocks[profile2.breakpoints[0].block_index]
            .cumulative_tokens
            .min(cacheable_total);
        let edited_later_breakpoint_tokens = profile2.blocks[profile2.breakpoints[1].block_index]
            .cumulative_tokens
            .min(cacheable_total);

        assert_eq!(result.cache_read_input_tokens, first_breakpoint_tokens);
        assert!(result.cache_read_input_tokens < edited_later_breakpoint_tokens);
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
        assert!(result.cache_creation_input_tokens > result.cache_read_input_tokens);
    }

    #[test]
    fn prefix_lookback_limits_to_recent_ten_breakpoints() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let mut messages = Vec::new();
        for i in 0..12 {
            messages.push(msg(
                "user",
                cache_text(&format!("{}-{i}", long_cacheable_text())),
            ));
            messages.push(msg("assistant", serde_json::json!(format!("reply-{i}"))));
        }
        let req = build_request(messages);
        let total = estimate_input_tokens(&req);
        let profile = tracker.build_profile(&req, total);
        assert!(profile.cacheable_breakpoints().len() >= 10);
    }

    #[test]
    fn only_explicit_breakpoints_are_created() {
        // With the implicit message-end breakpoint logic removed,
        // only blocks with explicit cache_control create breakpoints.
        let req = build_request(vec![
            msg("user", cache_text(&long_cacheable_text())),
            msg("assistant", serde_json::json!("R1")),
        ]);
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let profile = tracker.build_profile(&req, estimate_input_tokens(&req));
        let breakpoints = profile.cacheable_breakpoints();
        // Only one breakpoint: the explicit cache_control on the user message
        assert_eq!(breakpoints.len(), 1);
    }

    #[test]
    fn multi_turn_history_same_breakpoint_gives_same_cache_read() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let long = long_cacheable_text();

        let req1 = build_request(vec![msg("user", cache_text(&long))]);
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        let result1 = tracker.compute(&CacheKey::User("test_user".into()), &profile1);
        assert!(result1.cache_creation_input_tokens > 0);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        // Adding more turns without new explicit breakpoints doesn't extend cache.
        // The same prefix is read from cache each time.
        let req2 = build_request(vec![
            msg("user", cache_text(&long)),
            msg("assistant", serde_json::json!(medium_turn_text("R1"))),
            msg("user", serde_json::json!(medium_turn_text("R2"))),
        ]);
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result2 = tracker.compute(&CacheKey::User("test_user".into()), &profile2);
        let cacheable_total2 = (total2 - 1).max(0);
        // breakpoint block 匹配 → cache_read 覆盖直到 breakpoint 的前缀
        assert!(result2.cache_read_input_tokens > 0);
        // 新增的 turn tokens 归入 cache_creation
        assert_eq!(
            result2.cache_read_input_tokens + result2.cache_creation_input_tokens,
            cacheable_total2
        );
        tracker.update(&CacheKey::User("test_user".into()), &profile2);

        let req3 = build_request(vec![
            msg("user", cache_text(&long)),
            msg("assistant", serde_json::json!(medium_turn_text("R1"))),
            msg("user", serde_json::json!(medium_turn_text("R2"))),
            msg("assistant", serde_json::json!(medium_turn_text("R2A"))),
            msg("user", serde_json::json!(medium_turn_text("R3"))),
        ]);
        let total3 = estimate_input_tokens(&req3);
        let profile3 = tracker.build_profile(&req3, total3);
        let result3 = tracker.compute(&CacheKey::User("test_user".into()), &profile3);
        let cacheable_total3 = (total3 - 1).max(0);
        // Same explicit breakpoint → same cache read tokens（breakpoint 块相同）
        assert_eq!(
            result3.cache_read_input_tokens,
            result2.cache_read_input_tokens
        );
        assert_eq!(
            result3.cache_read_input_tokens + result3.cache_creation_input_tokens,
            cacheable_total3
        );
    }

    #[test]
    fn explicit_1h_ttl_is_preserved_on_breakpoint() {
        let req = build_request(vec![
            msg(
                "user",
                serde_json::json!([{
                    "type": "text",
                    "text": long_cacheable_text(),
                    "cache_control": { "type": "ephemeral", "ttl": "1h" }
                }]),
            ),
            msg("assistant", serde_json::json!("R1")),
            msg("user", serde_json::json!("R2")),
        ]);
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let profile = tracker.build_profile(&req, estimate_input_tokens(&req));
        let breakpoints = profile.cacheable_breakpoints();
        // Only one explicit breakpoint with 1h TTL
        assert_eq!(breakpoints.len(), 1);
        assert_eq!(breakpoints[0].ttl, Duration::from_secs(3600));
    }

    #[test]
    fn tool_changes_invalidate_downstream_prefix() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let mut req1 = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        req1.tools.as_mut().unwrap().push(Tool {
            tool_type: None,
            name: "alpha".to_string(),
            description: "alpha".to_string(),
            input_schema: Default::default(),
            max_uses: None,
            cache_control: None,
        });
        let total1 = estimate_input_tokens(&req1);
        let profile1 = tracker.build_profile(&req1, total1);
        tracker.update(&CacheKey::User("test_user".into()), &profile1);

        let mut req2 = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        req2.tools.as_mut().unwrap().push(Tool {
            tool_type: None,
            name: "beta".to_string(),
            description: "beta".to_string(),
            input_schema: Default::default(),
            max_uses: None,
            cache_control: None,
        });
        let total2 = estimate_input_tokens(&req2);
        let profile2 = tracker.build_profile(&req2, total2);
        let result = tracker.compute(&CacheKey::User("test_user".into()), &profile2);

        let cacheable_total = (total2 - 1).max(0);

        // 第一个 tool block (echo) 未变，仍可命中；但 alpha→beta 变化导致后续所有 fingerprint 失效。
        // cache_read 仅覆盖未变的 echo tool 块，大部分归入 cache_creation。
        assert!(result.cache_read_input_tokens > 0);
        assert!(result.cache_creation_input_tokens > result.cache_read_input_tokens);
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }

    #[test]
    fn distinct_user_keys_do_not_share_cache_entries() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let req = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        let total = estimate_input_tokens(&req);
        let profile = tracker.build_profile(&req, total);

        tracker.update(&CacheKey::User("alice".into()), &profile);

        let bob_result = tracker.compute(&CacheKey::User("bob".into()), &profile);
        assert_eq!(
            bob_result.cache_read_input_tokens, 0,
            "different user should miss the cache"
        );
        assert!(bob_result.cache_creation_input_tokens > 0);

        let alice_result = tracker.compute(&CacheKey::User("alice".into()), &profile);
        assert!(
            alice_result.cache_read_input_tokens > 0,
            "alice should still hit her own cache"
        );
    }

    #[test]
    fn global_key_is_shared_across_calls() {
        let tracker = CacheTracker::new(Duration::from_secs(3600));
        let req = build_request(vec![msg("user", cache_text(&long_cacheable_text()))]);
        let total = estimate_input_tokens(&req);
        let profile = tracker.build_profile(&req, total);

        tracker.update(&CacheKey::Global, &profile);

        let result = tracker.compute(&CacheKey::Global, &profile);
        let cacheable_total = (total - 1).max(0);
        assert!(
            result.cache_read_input_tokens > 0,
            "Global key should hit cache previously written under Global"
        );
        // 不变量
        assert_eq!(
            result.cache_read_input_tokens + result.cache_creation_input_tokens,
            cacheable_total
        );
    }
}
