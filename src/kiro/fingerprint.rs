//! 多维度设备指纹系统
#![allow(dead_code)]
//!
//! 模拟真实 Kiro IDE 客户端的完整环境特征，降低被检测风险。
//! 参考 CLIProxyAPIPlus 的实现。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// SDK 版本范围
const SDK_VERSIONS: &[&str] = &["1.0.20", "1.0.22", "1.0.24", "1.0.25", "1.0.27"];

/// Kiro IDE 版本范围
const KIRO_VERSIONS: &[&str] = &["0.3.0", "0.4.0", "0.5.0", "0.6.0", "0.7.0", "0.8.0"];

/// Node.js 版本范围
const NODE_VERSIONS: &[&str] = &["18.20.4", "20.18.0", "22.11.0", "22.21.1"];

/// 操作系统类型
const OS_TYPES: &[&str] = &["darwin", "win32", "linux"];

/// 操作系统版本（按类型分组）
const DARWIN_VERSIONS: &[&str] = &["24.0.0", "24.1.0", "24.2.0", "24.4.0", "24.6.0"];
const WIN32_VERSIONS: &[&str] = &["10.0.19045", "10.0.22621", "10.0.22631"];
const LINUX_VERSIONS: &[&str] = &["6.5.0", "6.8.0", "6.11.0"];

/// 语言偏好
const ACCEPT_LANGUAGES: &[&str] = &[
    "en-US,en;q=0.9",
    "en-GB,en;q=0.9",
    "zh-CN,zh;q=0.9,en;q=0.8",
    "ja-JP,ja;q=0.9,en;q=0.8",
    "de-DE,de;q=0.9,en;q=0.8",
    "fr-FR,fr;q=0.9,en;q=0.8",
];

/// 屏幕分辨率
const SCREEN_RESOLUTIONS: &[&str] = &[
    "1920x1080",
    "2560x1440",
    "3840x2160",
    "1440x900",
    "2560x1600",
    "3024x1964",
];

/// 颜色深度
const COLOR_DEPTHS: &[u8] = &[24, 30, 32];

/// CPU 核心数范围
const HARDWARE_CONCURRENCY_RANGE: (u8, u8) = (4, 32);

/// 时区偏移范围（分钟）
const TIMEZONE_OFFSET_RANGE: (i16, i16) = (-720, 720);

/// 设备指纹
///
/// 包含模拟真实 Kiro IDE 客户端的完整环境特征
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fingerprint {
    /// AWS SDK 版本
    pub sdk_version: String,

    /// 操作系统类型
    pub os_type: String,

    /// 操作系统版本
    pub os_version: String,

    /// Node.js 版本
    pub node_version: String,

    /// Kiro IDE 版本
    pub kiro_version: String,

    /// Kiro IDE 哈希（模拟）
    pub kiro_hash: String,

    /// 语言偏好
    pub accept_language: String,

    /// 屏幕分辨率
    pub screen_resolution: String,

    /// 颜色深度
    pub color_depth: u8,

    /// CPU 核心数
    pub hardware_concurrency: u8,

    /// 时区偏移（分钟）
    pub timezone_offset: i16,

    /// Machine ID（64 字符十六进制）
    pub machine_id: String,
}

impl Fingerprint {
    /// 基于种子生成确定性指纹
    ///
    /// 使用种子确保同一凭据始终生成相同的指纹，
    /// 避免每次请求都生成不同的指纹导致被检测。
    ///
    /// # Arguments
    /// * `seed` - 种子字符串（通常使用 refresh_token 或 machine_id）
    pub fn generate_from_seed(seed: &str) -> Self {
        // 使用种子生成确定性随机数
        let hash = sha256_bytes(seed);

        // 从哈希中提取各个字段的索引
        let sdk_idx = hash[0] as usize % SDK_VERSIONS.len();
        let kiro_idx = hash[1] as usize % KIRO_VERSIONS.len();
        let node_idx = hash[2] as usize % NODE_VERSIONS.len();
        let os_idx = hash[3] as usize % OS_TYPES.len();
        let lang_idx = hash[4] as usize % ACCEPT_LANGUAGES.len();
        let screen_idx = hash[5] as usize % SCREEN_RESOLUTIONS.len();
        let color_idx = hash[6] as usize % COLOR_DEPTHS.len();

        let os_type = OS_TYPES[os_idx];
        let os_version = match os_type {
            "darwin" => DARWIN_VERSIONS[hash[7] as usize % DARWIN_VERSIONS.len()],
            "win32" => WIN32_VERSIONS[hash[7] as usize % WIN32_VERSIONS.len()],
            "linux" => LINUX_VERSIONS[hash[7] as usize % LINUX_VERSIONS.len()],
            _ => DARWIN_VERSIONS[0],
        };

        // 生成 hardware_concurrency（4-32）
        let (min_cores, max_cores) = HARDWARE_CONCURRENCY_RANGE;
        let cores_range = max_cores - min_cores + 1;
        let hardware_concurrency = min_cores + (hash[8] % cores_range);

        // 生成 timezone_offset（-720 到 720 分钟）
        let (min_tz, max_tz) = TIMEZONE_OFFSET_RANGE;
        let tz_range = (max_tz - min_tz + 1) as u16;
        let tz_offset = hash[9] as u16 * 256 + hash[10] as u16;
        let timezone_offset = min_tz + (tz_offset % tz_range) as i16;

        // 生成 kiro_hash（模拟 SHA256）
        let kiro_hash = sha256_hex(&format!("kiro-{}-{}", KIRO_VERSIONS[kiro_idx], seed));

        // 生成 machine_id
        let machine_id = sha256_hex(&format!("machine-{}", seed));

        Self {
            sdk_version: SDK_VERSIONS[sdk_idx].to_string(),
            os_type: os_type.to_string(),
            os_version: os_version.to_string(),
            node_version: NODE_VERSIONS[node_idx].to_string(),
            kiro_version: KIRO_VERSIONS[kiro_idx].to_string(),
            kiro_hash,
            accept_language: ACCEPT_LANGUAGES[lang_idx].to_string(),
            screen_resolution: SCREEN_RESOLUTIONS[screen_idx].to_string(),
            color_depth: COLOR_DEPTHS[color_idx],
            hardware_concurrency,
            timezone_offset,
            machine_id,
        }
    }

    /// 构建 User-Agent 字符串
    pub fn user_agent(&self) -> String {
        format!(
            "aws-sdk-js/{} ua/2.1 os/{}#{} lang/js md/nodejs#{} api/codewhispererstreaming#{} m/E KiroIDE-{}-{}",
            self.sdk_version,
            self.os_type,
            self.os_version,
            self.node_version,
            self.sdk_version,
            self.kiro_version,
            self.machine_id
        )
    }

    /// 构建 x-amz-user-agent 字符串
    pub fn x_amz_user_agent(&self) -> String {
        format!(
            "aws-sdk-js/{} KiroIDE-{}-{}",
            self.sdk_version, self.kiro_version, self.machine_id
        )
    }

    /// 获取格式化的操作系统字符串（用于 User-Agent）
    pub fn os_string(&self) -> String {
        format!("{}#{}", self.os_type, self.os_version)
    }
}

/// SHA256 哈希（返回字节数组）
fn sha256_bytes(input: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hasher.finalize().into()
}

/// SHA256 哈希（返回十六进制字符串）
fn sha256_hex(input: &str) -> String {
    hex::encode(sha256_bytes(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fingerprint_deterministic() {
        // 相同种子应生成相同指纹
        let fp1 = Fingerprint::generate_from_seed("test_seed");
        let fp2 = Fingerprint::generate_from_seed("test_seed");

        assert_eq!(fp1.sdk_version, fp2.sdk_version);
        assert_eq!(fp1.os_type, fp2.os_type);
        assert_eq!(fp1.kiro_version, fp2.kiro_version);
        assert_eq!(fp1.machine_id, fp2.machine_id);
    }

    #[test]
    fn test_fingerprint_different_seeds() {
        // 不同种子应生成不同指纹
        let fp1 = Fingerprint::generate_from_seed("seed1");
        let fp2 = Fingerprint::generate_from_seed("seed2");

        // machine_id 一定不同
        assert_ne!(fp1.machine_id, fp2.machine_id);
    }

    #[test]
    fn test_user_agent_format() {
        let fp = Fingerprint::generate_from_seed("test");
        let ua = fp.user_agent();

        assert!(ua.contains("aws-sdk-js/"));
        assert!(ua.contains("KiroIDE-"));
        assert!(ua.contains("lang/js"));
        assert!(ua.contains("md/nodejs#"));
    }

    #[test]
    fn test_x_amz_user_agent_format() {
        let fp = Fingerprint::generate_from_seed("test");
        let amz_ua = fp.x_amz_user_agent();

        assert!(amz_ua.contains("aws-sdk-js/"));
        assert!(amz_ua.contains("KiroIDE-"));
    }

    #[test]
    fn test_machine_id_length() {
        let fp = Fingerprint::generate_from_seed("test");
        assert_eq!(fp.machine_id.len(), 64);
    }

    #[test]
    fn test_hardware_concurrency_range() {
        for i in 0..100 {
            let fp = Fingerprint::generate_from_seed(&format!("test_{}", i));
            assert!(fp.hardware_concurrency >= 4);
            assert!(fp.hardware_concurrency <= 32);
        }
    }

    #[test]
    fn test_timezone_offset_range() {
        for i in 0..100 {
            let fp = Fingerprint::generate_from_seed(&format!("test_{}", i));
            assert!(fp.timezone_offset >= -720);
            assert!(fp.timezone_offset <= 720);
        }
    }

    #[test]
    fn test_os_version_matches_type() {
        // darwin 类型应该有 darwin 版本
        let fp = Fingerprint {
            sdk_version: "1.0.27".to_string(),
            os_type: "darwin".to_string(),
            os_version: "24.6.0".to_string(),
            node_version: "22.21.1".to_string(),
            kiro_version: "0.8.0".to_string(),
            kiro_hash: "test".to_string(),
            accept_language: "en-US".to_string(),
            screen_resolution: "1920x1080".to_string(),
            color_depth: 24,
            hardware_concurrency: 8,
            timezone_offset: -480,
            machine_id: "a".repeat(64),
        };

        let os_str = fp.os_string();
        assert!(os_str.starts_with("darwin#"));
    }
}
