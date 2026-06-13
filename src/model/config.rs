use anyhow::Context;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TlsBackend {
    #[default]
    Rustls,
    NativeTls,
}

/// Prompt Cache 记账模式
///
/// - `Upstream`: 直接采用上游（contextUsageEvent / 未来的 messageMetadataEvent.tokenUsage）
///   计算的 input_tokens，不输出 cache_creation/cache_read 字段。
/// - `Simulated`: 在 `Upstream` 总量基础上叠加本地 `CacheTracker` 的 cache 记账，
///   输出 cache_creation/cache_read 以及 5m/1h 拆分。
/// - `Off`: 不进行 cache 记账；input_tokens 使用本地估算。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum PromptCacheMode {
    Upstream,
    #[default]
    Simulated,
    Off,
}

/// 自定义反序列化：兼容老版本 `promptCacheAccountingEnabled: bool` 字段
///
/// - `bool` true → `Simulated`，false → `Off`
/// - `string` 取 "upstream" / "simulated" / "off"，其它值返回 unknown_variant 错误
fn deserialize_prompt_cache_mode<'de, D>(deserializer: D) -> Result<PromptCacheMode, D::Error>
where
    D: Deserializer<'de>,
{
    struct PromptCacheModeVisitor;

    impl<'de> Visitor<'de> for PromptCacheModeVisitor {
        type Value = PromptCacheMode;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a bool or one of \"upstream\", \"simulated\", \"off\"")
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(if value {
                PromptCacheMode::Simulated
            } else {
                PromptCacheMode::Off
            })
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            match value {
                "upstream" => Ok(PromptCacheMode::Upstream),
                "simulated" => Ok(PromptCacheMode::Simulated),
                "off" => Ok(PromptCacheMode::Off),
                other => Err(de::Error::unknown_variant(
                    other,
                    &["upstream", "simulated", "off"],
                )),
            }
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            self.visit_str(&value)
        }
    }

    deserializer.deserialize_any(PromptCacheModeVisitor)
}

/// 模型配置条目（用于 config.json 中的 models 数组）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub id: String,
    pub display_name: String,
    pub created_at: i64,
    pub kiro_model_id: String,
}

/// KNA 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_region")]
    pub region: String,

    /// API Region（用于 API 请求），未配置时回退到 region
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_region: Option<String>,

    #[serde(default = "default_kiro_version")]
    pub kiro_version: String,

    #[serde(default)]
    pub machine_id: Option<String>,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(default = "default_system_version")]
    pub system_version: String,

    #[serde(default = "default_node_version")]
    pub node_version: String,

    #[serde(default = "default_tls_backend")]
    pub tls_backend: TlsBackend,

    /// 外部 count_tokens API 地址（可选）
    #[serde(default)]
    pub count_tokens_api_url: Option<String>,

    /// count_tokens API 密钥（可选）
    #[serde(default)]
    pub count_tokens_api_key: Option<String>,

    /// count_tokens API 认证类型（可选，"x-api-key" 或 "bearer"，默认 "x-api-key"）
    #[serde(default = "default_count_tokens_auth_type")]
    pub count_tokens_auth_type: String,

    /// HTTP 代理地址（可选）
    /// 支持格式: http://host:port, https://host:port, socks5://host:port
    #[serde(default)]
    pub proxy_url: Option<String>,

    /// 代理认证用户名（可选）
    #[serde(default)]
    pub proxy_username: Option<String>,

    /// 代理认证密码（可选）
    #[serde(default)]
    pub proxy_password: Option<String>,

    /// Admin API 密钥（可选，启用 Admin API 功能）
    #[serde(default)]
    pub admin_api_key: Option<String>,

    /// 单个凭据的目标请求速率（RPM，每分钟请求数）
    ///
    /// 用于凭据级节流/分流：当某个凭据短时间内请求过密时，优先将流量分配到其他可用凭据，
    /// 从而减少上游 429 的概率。
    ///
    /// - `None` 或 `0`: 使用内置默认节流策略
    /// - `>0`: 将最小/最大请求间隔固定为 `60_000 / rpm` 毫秒
    #[serde(default)]
    pub credential_rpm: Option<u32>,

    /// 输入压缩配置
    #[serde(default)]
    pub compression: CompressionConfig,

    /// 响应关键词改写配置
    #[serde(default)]
    pub rewriter: crate::anthropic::rewriter::RewriterConfig,

    /// Prompt Cache TTL（秒），默认 300 秒
    #[serde(default = "default_prompt_cache_ttl_seconds")]
    pub prompt_cache_ttl_seconds: u64,

    /// 本地 Prompt Cache 记账模式，默认 `Simulated`
    ///
    /// 兼容旧字段 `promptCacheAccountingEnabled`：
    /// - `true`  → `Simulated`
    /// - `false` → `Off`
    #[serde(
        default,
        alias = "promptCacheAccountingEnabled",
        deserialize_with = "deserialize_prompt_cache_mode"
    )]
    pub prompt_cache_mode: PromptCacheMode,

    /// 默认端点名称（凭据未显式指定 endpoint 时使用）
    #[serde(default = "default_endpoint")]
    pub default_endpoint: String,

    /// 是否开启非流式响应的 thinking 块提取（默认 true）
    ///
    /// 启用后，非流式响应中的 `<thinking>...</thinking>` 标签会被解析为
    /// 独立的 `{"type": "thinking", ...}` 内容块,与流式响应行为一致。
    #[serde(default = "default_extract_thinking")]
    pub extract_thinking: bool,

    /// 是否启用凭据冷却机制（默认 true）
    ///
    /// 禁用后，429 限流响应不会触发凭据冷却，仍会尝试故障转移到其他凭据。
    #[serde(default = "default_true")]
    pub enable_credential_cooldown: bool,

    /// 是否启用速率限制节流（默认 true）
    ///
    /// 控制内置 RateLimitConfig 策略（每日最大请求数、请求间隔、指数退避）。
    /// 禁用后凭据级主动节流完全放行，不再因每日上限/最小间隔/退避而等待或分流；
    /// 不影响上游 429 触发的冷却（由 `enable_credential_cooldown` 控制）。
    #[serde(default = "default_true")]
    pub enable_rate_limit: bool,

    /// 是否启用粘性路由（默认 true）
    ///
    /// 启用后，同一 session（user_id）的请求会尽量路由到同一凭据，
    /// 不同 session 之间轮询分配凭据。当凭据触发 429 时，会自动将该 session
    /// 迁移到其他可用凭据。禁用后退化为纯负载均衡模式（无亲和性）。
    #[serde(default = "default_true")]
    pub enable_sticky_routing: bool,

    /// 余额不足时是否自动禁用凭据（默认 true）
    ///
    /// 禁用后，余额初始化检测到余额不足时不会自动禁用凭据，仅记录警告日志。
    #[serde(default = "default_true")]
    pub auto_disable_insufficient_balance: bool,

    /// Token 刷新失败时是否自动禁用凭据（默认 true）
    ///
    /// 禁用后，Token 刷新连续失败达到阈值时不会自动禁用凭据，仅记录警告日志。
    /// 注意：invalid_grant 错误（凭据已失效）始终会禁用凭据，不受此开关影响。
    #[serde(default = "default_true")]
    pub auto_disable_refresh_failure: bool,

    /// 上游返回 403 时是否自动禁用凭据
    #[serde(default = "default_true")]
    pub auto_disable_on_forbidden: bool,

    #[serde(default = "crate::anthropic::model_mapper::default_models")]
    pub models: Vec<ModelConfig>,

    /// 端点特定的配置
    ///
    /// 键为端点名（如 "ide" / "cli"），值为该端点自由定义的参数对象。
    /// 未在此表出现的端点沿用实现内置默认值。
    #[serde(default)]
    pub endpoints: HashMap<String, serde_json::Value>,

    /// 配置文件路径（运行时元数据，不写入 JSON）
    #[serde(skip)]
    config_path: Option<PathBuf>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_region() -> String {
    "us-east-1".to_string()
}

fn default_kiro_version() -> String {
    "0.11.107".to_string()
}

fn default_system_version() -> String {
    const SYSTEM_VERSIONS: &[&str] = &["darwin#24.6.0", "win32#10.0.22631"];
    SYSTEM_VERSIONS[fastrand::usize(..SYSTEM_VERSIONS.len())].to_string()
}

fn default_node_version() -> String {
    "22.22.0".to_string()
}

fn default_count_tokens_auth_type() -> String {
    "x-api-key".to_string()
}

fn default_endpoint() -> String {
    "ide".to_string()
}

fn default_prompt_cache_ttl_seconds() -> u64 {
    300
}

fn default_tls_backend() -> TlsBackend {
    TlsBackend::Rustls
}

fn default_true() -> bool {
    true
}

fn default_thinking_strategy() -> String {
    "discard".to_string()
}

fn default_4000() -> usize {
    4000
}

fn default_tool_definition_min_description_chars() -> usize {
    50
}

fn default_tool_name_max_chars() -> usize {
    63
}

fn default_max_request_body_bytes() -> usize {
    // 上游对请求体大小存在硬性限制（实测约 5MiB 左右会触发 400），
    // 这里默认设置为 4.5MiB 留出安全余量。
    4_718_592
}

fn default_adaptive_compression_max_iters() -> usize {
    32
}

/// 输入压缩配置
///
/// 控制请求体在协议转换后、发送到上游前的多层压缩策略。
/// 所有阈值均可通过配置文件调整，默认开启。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionConfig {
    /// 总开关，默认 true
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 空白压缩（连续空行、行尾空格），默认 true
    #[serde(default = "default_true")]
    pub whitespace_compression: bool,
    /// thinking 块处理策略: "discard" | "truncate" | "keep"
    #[serde(default = "default_thinking_strategy")]
    pub thinking_strategy: String,
    /// 工具描述截断阈值（字符数），转换时硬截断，默认 4000
    #[serde(default = "default_4000")]
    pub tool_description_max_chars: usize,
    /// 工具定义总大小超阈值后的 schema 简化 + desc 截断开关，默认 true
    #[serde(default = "default_true")]
    pub tool_definition_compression: bool,
    /// 工具定义压缩时 description 最少保留字符数，默认 50
    #[serde(default = "default_tool_definition_min_description_chars")]
    pub tool_definition_min_description_chars: usize,
    /// Kiro 工具名最大长度（字符数，0=不缩短），默认 63
    #[serde(default = "default_tool_name_max_chars")]
    pub tool_name_max_chars: usize,
    /// 请求体最大字节数，超过则触发自适应压缩（0 = 不限制）
    #[serde(default = "default_max_request_body_bytes")]
    pub max_request_body_bytes: usize,
    /// 请求体超限后的自适应压缩，默认 true
    #[serde(default = "default_true")]
    pub adaptive_compression: bool,
    /// 自适应压缩最大迭代次数，默认 32
    #[serde(default = "default_adaptive_compression_max_iters")]
    pub adaptive_compression_max_iters: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            whitespace_compression: true,
            thinking_strategy: default_thinking_strategy(),
            tool_description_max_chars: default_4000(),
            tool_definition_compression: true,
            tool_definition_min_description_chars: default_tool_definition_min_description_chars(),
            tool_name_max_chars: default_tool_name_max_chars(),
            max_request_body_bytes: default_max_request_body_bytes(),
            adaptive_compression: true,
            adaptive_compression_max_iters: default_adaptive_compression_max_iters(),
        }
    }
}

fn default_extract_thinking() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            region: default_region(),
            api_region: None,
            kiro_version: default_kiro_version(),
            machine_id: None,
            api_key: None,
            system_version: default_system_version(),
            node_version: default_node_version(),
            tls_backend: default_tls_backend(),
            count_tokens_api_url: None,
            count_tokens_api_key: None,
            count_tokens_auth_type: default_count_tokens_auth_type(),
            proxy_url: None,
            proxy_username: None,
            proxy_password: None,
            admin_api_key: None,
            credential_rpm: None,
            compression: CompressionConfig::default(),
            rewriter: crate::anthropic::rewriter::RewriterConfig::default(),
            prompt_cache_ttl_seconds: default_prompt_cache_ttl_seconds(),
            prompt_cache_mode: PromptCacheMode::default(),
            extract_thinking: default_extract_thinking(),
            enable_credential_cooldown: default_true(),
            enable_rate_limit: default_true(),
            enable_sticky_routing: default_true(),
            auto_disable_insufficient_balance: default_true(),
            auto_disable_refresh_failure: default_true(),
            auto_disable_on_forbidden: default_true(),
            default_endpoint: default_endpoint(),
            models: crate::anthropic::model_mapper::default_models(),
            endpoints: HashMap::new(),
            config_path: None,
        }
    }
}

impl Config {
    /// 获取默认配置文件路径
    pub fn default_config_path() -> &'static str {
        "config.json"
    }

    /// 获取有效的 API Region（用于 API 请求）
    /// 优先使用 api_region，未配置时回退到 region
    pub fn effective_api_region(&self) -> &str {
        self.api_region.as_deref().unwrap_or(&self.region)
    }

    /// 从文件加载配置
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            // 配置文件不存在，返回默认配置
            return Ok(Self {
                config_path: Some(path.to_path_buf()),
                ..Default::default()
            });
        }

        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        config.config_path = Some(path.to_path_buf());
        Ok(config)
    }

    /// 获取配置文件路径（如果有）
    #[cfg(test)]
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// 将当前配置写回原始配置文件
    pub fn save(&self) -> anyhow::Result<()> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("配置文件路径未知，无法保存配置"))?;

        let content = serde_json::to_string_pretty(self).context("序列化配置失败")?;
        fs::write(path, content)
            .with_context(|| format!("写入配置文件失败: {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_deserializes_prompt_cache_accounting_false() {
        let config: Config = serde_json::from_str(r#"{"promptCacheAccountingEnabled":false}"#)
            .expect("config should deserialize");
        assert_eq!(config.prompt_cache_mode, PromptCacheMode::Off);
    }

    #[test]
    fn test_config_deserializes_prompt_cache_accounting_true_as_simulated() {
        let config: Config = serde_json::from_str(r#"{"promptCacheAccountingEnabled":true}"#)
            .expect("config should deserialize");
        assert_eq!(config.prompt_cache_mode, PromptCacheMode::Simulated);
    }

    #[test]
    fn test_config_deserializes_prompt_cache_mode_upstream() {
        let config: Config = serde_json::from_str(r#"{"promptCacheMode":"upstream"}"#)
            .expect("config should deserialize");
        assert_eq!(config.prompt_cache_mode, PromptCacheMode::Upstream);
    }

    #[test]
    fn test_config_deserializes_prompt_cache_mode_off() {
        let config: Config = serde_json::from_str(r#"{"promptCacheMode":"off"}"#)
            .expect("config should deserialize");
        assert_eq!(config.prompt_cache_mode, PromptCacheMode::Off);
    }

    #[test]
    fn test_config_deserializes_prompt_cache_mode_simulated() {
        let config: Config = serde_json::from_str(r#"{"promptCacheMode":"simulated"}"#)
            .expect("config should deserialize");
        assert_eq!(config.prompt_cache_mode, PromptCacheMode::Simulated);
    }

    #[test]
    fn test_config_default_prompt_cache_mode_is_simulated() {
        let config: Config = serde_json::from_str("{}").expect("config should deserialize");
        assert_eq!(config.prompt_cache_mode, PromptCacheMode::Simulated);
    }

    #[test]
    fn test_config_rejects_unknown_prompt_cache_mode_string() {
        let result: Result<Config, _> = serde_json::from_str(r#"{"promptCacheMode":"bogus"}"#);
        assert!(result.is_err(), "unknown variant should be rejected");
    }
}
