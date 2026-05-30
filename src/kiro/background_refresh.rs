//! 后台 Token 刷新模块
#![allow(dead_code)]
//!
//! 独立后台任务定期检查并刷新即将过期的 Token，
//! 避免请求时的刷新延迟。
//! 参考 CLIProxyAPIPlus 的实现。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::interval;

/// 默认检查间隔（秒）
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 60;

/// 默认批处理大小
const DEFAULT_BATCH_SIZE: usize = 50;

/// 默认并发数
const DEFAULT_CONCURRENCY: usize = 10;

/// 默认提前刷新时间（分钟）
/// Token 在过期前多少分钟开始刷新
const DEFAULT_REFRESH_BEFORE_EXPIRY_MINS: i64 = 15;

/// 后台刷新配置
#[derive(Debug, Clone)]
pub struct BackgroundRefreshConfig {
    /// 检查间隔（秒），必须 > 0
    pub check_interval_secs: u64,

    /// 批处理大小，必须 > 0
    pub batch_size: usize,

    /// 并发数，必须 > 0
    pub concurrency: usize,

    /// 提前刷新时间（分钟）
    pub refresh_before_expiry_mins: i64,
}

impl BackgroundRefreshConfig {
    /// 校验配置有效性
    ///
    /// # Returns
    /// - `Ok(())` - 配置有效
    /// - `Err(String)` - 配置无效，返回错误描述
    pub fn validate(&self) -> Result<(), String> {
        if self.check_interval_secs == 0 {
            return Err("check_interval_secs 必须大于 0".to_string());
        }
        if self.batch_size == 0 {
            return Err("batch_size 必须大于 0".to_string());
        }
        if self.concurrency == 0 {
            return Err("concurrency 必须大于 0".to_string());
        }
        Ok(())
    }
}

impl Default for BackgroundRefreshConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: DEFAULT_CHECK_INTERVAL_SECS,
            batch_size: DEFAULT_BATCH_SIZE,
            concurrency: DEFAULT_CONCURRENCY,
            refresh_before_expiry_mins: DEFAULT_REFRESH_BEFORE_EXPIRY_MINS,
        }
    }
}

/// 后台刷新器
///
/// 管理后台 Token 刷新任务
pub struct BackgroundRefresher {
    config: BackgroundRefreshConfig,
    running: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
}

impl BackgroundRefresher {
    /// 创建新的后台刷新器
    pub fn new(config: BackgroundRefreshConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
        }
    }

    /// 使用默认配置创建后台刷新器
    pub fn with_defaults() -> Self {
        Self::new(BackgroundRefreshConfig::default())
    }

    /// 启动后台刷新任务
    ///
    /// # Arguments
    /// * `refresh_fn` - 刷新函数，接收凭据 ID，返回是否成功
    /// * `get_expiring_ids_fn` - 获取即将过期的凭据 ID 列表
    ///
    /// # Returns
    /// - `Ok(())` - 启动成功
    /// - `Err(String)` - 配置无效或已在运行
    pub fn start<F, G>(&self, refresh_fn: F, get_expiring_ids_fn: G) -> Result<(), String>
    where
        F: Fn(u64) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>
            + Send
            + Sync
            + 'static,
        G: Fn(i64) -> Vec<u64> + Send + Sync + 'static,
    {
        // P1 修复：启动前校验配置，避免 panic/hang
        if let Err(e) = self.config.validate() {
            tracing::error!("后台刷新器配置无效: {}", e);
            return Err(e);
        }

        if self.running.swap(true, Ordering::SeqCst) {
            tracing::warn!("后台刷新器已在运行");
            return Err("后台刷新器已在运行".to_string());
        }

        let config = self.config.clone();
        let running = Arc::clone(&self.running);
        let shutdown_notify = Arc::clone(&self.shutdown_notify);
        let refresh_fn = Arc::new(refresh_fn);

        tokio::spawn(async move {
            tracing::info!(
                interval_secs = %config.check_interval_secs,
                batch_size = %config.batch_size,
                concurrency = %config.concurrency,
                "后台 Token 刷新器已启动"
            );

            let mut check_interval = interval(Duration::from_secs(config.check_interval_secs));

            loop {
                tokio::select! {
                    _ = check_interval.tick() => {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }

                        // 获取即将过期的凭据
                        let expiring_ids = get_expiring_ids_fn(config.refresh_before_expiry_mins);

                        if expiring_ids.is_empty() {
                            tracing::debug!("没有需要刷新的 Token");
                            continue;
                        }

                        tracing::info!("发现 {} 个即将过期的 Token，开始刷新", expiring_ids.len());

                        // 批量刷新
                        let mut success_count = 0;
                        let mut fail_count = 0;

                        for chunk in expiring_ids.chunks(config.batch_size) {
                            let semaphore = Arc::new(tokio::sync::Semaphore::new(config.concurrency));
                            let mut handles = Vec::new();

                            for &id in chunk {
                                let permit = semaphore.clone().acquire_owned().await;
                                let refresh_fn = Arc::clone(&refresh_fn);

                                let handle = tokio::spawn(async move {
                                    let _permit = permit;
                                    refresh_fn(id).await
                                });
                                handles.push(handle);
                            }

                            for handle in handles {
                                match handle.await {
                                    Ok(true) => success_count += 1,
                                    Ok(false) => fail_count += 1,
                                    Err(e) => {
                                        tracing::warn!("刷新任务 panic: {}", e);
                                        fail_count += 1;
                                    }
                                }
                            }
                        }

                        tracing::info!(
                            success = %success_count,
                            failed = %fail_count,
                            "后台 Token 刷新完成"
                        );
                    }
                    _ = shutdown_notify.notified() => {
                        tracing::info!("后台 Token 刷新器收到关闭信号");
                        break;
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            tracing::info!("后台 Token 刷新器已停止");
        });

        Ok(())
    }

    /// 停止后台刷新任务
    pub fn stop(&self) {
        if self.running.load(Ordering::SeqCst) {
            self.running.store(false, Ordering::SeqCst);
            self.shutdown_notify.notify_one();
            tracing::info!("已发送后台刷新器停止信号");
        }
    }

    /// 检查是否正在运行
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 获取配置
    pub fn config(&self) -> &BackgroundRefreshConfig {
        &self.config
    }
}

impl Drop for BackgroundRefresher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 刷新结果
#[derive(Debug, Clone)]
pub struct RefreshResult {
    /// 凭据 ID
    pub credential_id: u64,

    /// 是否成功
    pub success: bool,

    /// 是否使用了降级方案
    pub used_fallback: bool,

    /// 错误信息（如果失败）
    pub error: Option<String>,

    /// 新的过期时间（如果成功）
    pub new_expires_at: Option<String>,
}

impl RefreshResult {
    /// 创建成功结果
    pub fn success(credential_id: u64, new_expires_at: String) -> Self {
        Self {
            credential_id,
            success: true,
            used_fallback: false,
            error: None,
            new_expires_at: Some(new_expires_at),
        }
    }

    /// 创建失败结果
    pub fn failure(credential_id: u64, error: String) -> Self {
        Self {
            credential_id,
            success: false,
            used_fallback: false,
            error: Some(error),
            new_expires_at: None,
        }
    }

    /// 创建降级结果（刷新失败但使用现有 Token）
    pub fn fallback(credential_id: u64, existing_expires_at: String) -> Self {
        Self {
            credential_id,
            success: true,
            used_fallback: true,
            error: None,
            new_expires_at: Some(existing_expires_at),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_background_refresher_new() {
        let refresher = BackgroundRefresher::with_defaults();
        assert!(!refresher.is_running());
    }

    #[test]
    fn test_refresh_result_success() {
        let result = RefreshResult::success(1, "2025-01-01T00:00:00Z".to_string());
        assert!(result.success);
        assert!(!result.used_fallback);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_refresh_result_failure() {
        let result = RefreshResult::failure(1, "Token expired".to_string());
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_refresh_result_fallback() {
        let result = RefreshResult::fallback(1, "2025-01-01T00:00:00Z".to_string());
        assert!(result.success);
        assert!(result.used_fallback);
    }

    #[tokio::test]
    async fn test_background_refresher_stop() {
        let refresher = BackgroundRefresher::with_defaults();

        // 启动一个空的刷新任务
        let _ = refresher.start(|_id| Box::pin(async { true }), |_mins| vec![]);

        // 等待启动
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(refresher.is_running());

        // 停止
        refresher.stop();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!refresher.is_running());
    }
}
