//! Commit-Confirm 事务管理器与独立看门狗 (Fail-Safe Watchdog)

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};

use crate::error::ApiError;
use types::system::{IpConfig, NetworkChangeOperation, OperationConfirmResult, OperationStatus};

/// 试运行默认超时时长（毫秒）：60 秒
pub const DEFAULT_TRIAL_TIMEOUT_MS: i64 = 60_000;

/// 持久化与临时快照目录候选
const SNAPSHOT_DIRS: &[&str] = &[
    "/etc/heimdall/network",
    "/var/lib/heimdall/network",
    "/run/heimdall",
    "/tmp/heimdall",
];

/// 网络操作快照结构（用于掉电持久化）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OperationSnapshot {
    pub operation: NetworkChangeOperation,
    pub manager_type: String,
}

#[derive(Debug)]
pub struct NetworkOperationManager {
    current: Arc<RwLock<Option<NetworkChangeOperation>>>,
    watchdog_cancel: Arc<RwLock<Option<oneshot::Sender<()>>>>,
}

impl Default for NetworkOperationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkOperationManager {
    pub fn new() -> Self {
        Self {
            current: Arc::new(RwLock::new(None)),
            watchdog_cancel: Arc::new(RwLock::new(None)),
        }
    }

    /// 获取当前未决的操作
    pub async fn get_pending(&self) -> Option<NetworkChangeOperation> {
        let guard = self.current.read().await;
        guard.clone()
    }

    /// 启动试运行事务并激活看门狗
    ///
    /// `rollback_fn`: 当看门狗超时时触发的恢复函数
    pub async fn start_trial<F, Fut>(
        &self,
        interface_name: &str,
        old_config: IpConfig,
        new_config: IpConfig,
        new_access_url: Option<String>,
        manager_type: &str,
        rollback_fn: F,
    ) -> Result<NetworkChangeOperation, ApiError>
    where
        F: FnOnce(String, IpConfig) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        let mut current_guard = self.current.write().await;

        // 检查是否有仍在进行中的未决事务
        if let Some(existing) = current_guard.as_ref() {
            if existing.status == OperationStatus::PendingConfirm {
                let now = chrono::Utc::now().timestamp_millis();
                if now < existing.confirm_deadline_ms {
                    return Err(ApiError::NetworkPendingOperation);
                }
            }
        }

        let now = chrono::Utc::now().timestamp_millis();
        let deadline = now + DEFAULT_TRIAL_TIMEOUT_MS;
        let op_id = uuid::Uuid::new_v4().to_string();

        let operation = NetworkChangeOperation {
            id: op_id.clone(),
            status: OperationStatus::PendingConfirm,
            interface_name: interface_name.to_string(),
            old_config: old_config.clone(),
            new_config: new_config.clone(),
            created_at: now,
            confirm_deadline_ms: deadline,
            new_access_url,
        };

        // 持久化快照到磁盘（防重启/掉电）
        let snapshot = OperationSnapshot {
            operation: operation.clone(),
            manager_type: manager_type.to_string(),
        };
        save_snapshot_atomic(&snapshot).await?;

        // 停止之前的看门狗
        let mut cancel_guard = self.watchdog_cancel.write().await;
        if let Some(old_tx) = cancel_guard.take() {
            let _ = old_tx.send(());
        }

        // 启动新的独立看门狗定时器
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        *cancel_guard = Some(cancel_tx);
        *current_guard = Some(operation.clone());

        let current_clone = Arc::clone(&self.current);
        let iface_name = interface_name.to_string();
        let old_cfg = old_config.clone();

        tokio::spawn(async move {
            let sleep_duration = std::time::Duration::from_millis(DEFAULT_TRIAL_TIMEOUT_MS as u64);
            tokio::select! {
                _ = &mut cancel_rx => {
                    tracing::info!("网络操作看门狗收到终止信号（已被确认或手动取消）");
                }
                _ = tokio::time::sleep(sleep_duration) => {
                    tracing::warn!("【防失联警告】网络变更试运行超时（60s未收到确认），启动原子回滚！");

                    let mut guard = current_clone.write().await;
                    if let Some(op) = guard.as_mut() {
                        if op.status == OperationStatus::PendingConfirm {
                            op.status = OperationStatus::Restoring;
                            drop(guard);

                            if let Err(e) = rollback_fn(iface_name.clone(), old_cfg.clone()).await {
                                tracing::error!("看门狗自动回滚网络配置失败: {e}");
                            } else {
                                tracing::info!("看门狗自动回滚网络配置成功，旧配置已恢复！");
                            }

                            let mut guard = current_clone.write().await;
                            if let Some(op) = guard.as_mut() {
                                op.status = OperationStatus::Restored;
                            }
                            // 移除磁盘快照
                            remove_snapshot(&iface_name).await;
                        }
                    }
                }
            }
        });

        Ok(operation)
    }

    /// 确认网络操作（固化生效）
    pub async fn confirm(&self, operation_id: &str) -> Result<OperationConfirmResult, ApiError> {
        let mut current_guard = self.current.write().await;
        let op = current_guard
            .as_mut()
            .ok_or(ApiError::NetworkOperationExpired)?;

        if op.id != operation_id {
            return Err(ApiError::NetworkOperationExpired);
        }

        if op.status != OperationStatus::PendingConfirm {
            return Err(ApiError::NetworkOperationExpired);
        }

        // 停止看门狗
        let mut cancel_guard = self.watchdog_cancel.write().await;
        if let Some(tx) = cancel_guard.take() {
            let _ = tx.send(());
        }

        let now = chrono::Utc::now().timestamp_millis();
        op.status = OperationStatus::Confirmed;
        remove_snapshot(&op.interface_name).await;

        Ok(OperationConfirmResult {
            status: OperationStatus::Confirmed,
            confirmed_at: Some(now),
        })
    }

    /// 手动放弃试运行并立即回滚
    pub async fn cancel<F, Fut>(
        &self,
        operation_id: &str,
        rollback_fn: F,
    ) -> Result<OperationConfirmResult, ApiError>
    where
        F: FnOnce(String, IpConfig) -> Fut,
        Fut: std::future::Future<Output = Result<(), ApiError>>,
    {
        let mut current_guard = self.current.write().await;
        let op = current_guard
            .as_mut()
            .ok_or(ApiError::NetworkOperationExpired)?;

        if op.id != operation_id {
            return Err(ApiError::NetworkOperationExpired);
        }

        // 停止看门狗
        let mut cancel_guard = self.watchdog_cancel.write().await;
        if let Some(tx) = cancel_guard.take() {
            let _ = tx.send(());
        }

        let iface = op.interface_name.clone();
        let old_config = op.old_config.clone();
        op.status = OperationStatus::Restoring;
        drop(current_guard);

        rollback_fn(iface.clone(), old_config).await?;

        let mut current_guard = self.current.write().await;
        if let Some(op) = current_guard.as_mut() {
            op.status = OperationStatus::Restored;
        }
        remove_snapshot(&iface).await;

        Ok(OperationConfirmResult {
            status: OperationStatus::Restored,
            confirmed_at: None,
        })
    }

    /// 中止并清理试运行事务（下发失败时立即恢复状态，避免系统死锁在 Pending 状态）
    pub async fn abort_trial(&self, operation_id: &str) {
        let mut current_guard = self.current.write().await;
        if let Some(op) = current_guard.as_ref() {
            if op.id == operation_id {
                let mut cancel_guard = self.watchdog_cancel.write().await;
                if let Some(tx) = cancel_guard.take() {
                    let _ = tx.send(());
                }
                let iface = op.interface_name.clone();
                *current_guard = None;
                drop(current_guard);
                remove_snapshot(&iface).await;
            }
        }
    }
}

/// 获取快照存储目录（优先持久化目录，保证掉电与重启安全）
fn get_snapshot_dir() -> PathBuf {
    for dir_str in SNAPSHOT_DIRS {
        let p = Path::new(dir_str);
        if p.exists() || std::fs::create_dir_all(p).is_ok() {
            return p.to_path_buf();
        }
    }
    PathBuf::from("/tmp")
}

fn get_snapshot_path(iface: &str) -> PathBuf {
    get_snapshot_dir().join(format!("network_backup_{iface}.json"))
}

/// 原子化持久化快照（write .tmp -> sync_all -> rename）
pub async fn save_snapshot_atomic(snapshot: &OperationSnapshot) -> Result<(), ApiError> {
    let target = get_snapshot_path(&snapshot.operation.interface_name);
    let tmp = target.with_extension(format!("tmp.{}", std::process::id()));

    let content = serde_json::to_string_pretty(snapshot)
        .map_err(|e| ApiError::Internal(format!("序列化网络快照失败: {e}")))?;

    // 写入临时文件并刷盘
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("创建网络快照临时文件失败: {e}")))?;
    file.write_all(content.as_bytes())
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("写入网络快照失败: {e}")))?;
    file.sync_all()
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("刷盘网络快照失败: {e}")))?;
    drop(file);

    // 原子重命名覆盖
    tokio::fs::rename(&tmp, &target)
        .await
        .map_err(|e| ApiError::NetworkFailed(format!("重命名网络快照失败: {e}")))?;

    Ok(())
}

/// 移除快照
pub async fn remove_snapshot(iface: &str) {
    let filename = format!("network_backup_{iface}.json");
    for dir_str in SNAPSHOT_DIRS {
        let path = Path::new(dir_str).join(&filename);
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// 冷启动恢复检查：若系统刚开机发现残留未决快照，返回需恢复的快照列表
pub async fn load_pending_snapshots() -> Vec<OperationSnapshot> {
    let mut results = Vec::new();
    for dir_str in SNAPSHOT_DIRS {
        let dir = Path::new(dir_str);
        if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| s.starts_with("network_backup_") && s.ends_with(".json"))
                    .unwrap_or(false)
                {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        if let Ok(snapshot) = serde_json::from_str::<OperationSnapshot>(&content) {
                            results.push(snapshot);
                        }
                    }
                }
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::system::IpMethod;

    fn sample_config(ip: &str) -> IpConfig {
        IpConfig {
            method: IpMethod::Static,
            address: Some(ip.to_string()),
            prefix: Some(24),
            gateway: Some("192.168.1.1".to_string()),
            dns: vec!["8.8.8.8".to_string()],
            metric: Some(100),
        }
    }

    #[tokio::test]
    async fn test_operation_manager_confirm_lifecycle() {
        let mgr = NetworkOperationManager::new();
        let old_cfg = sample_config("192.168.1.10");
        let new_cfg = sample_config("192.168.1.20");

        let op = mgr
            .start_trial(
                "eth0",
                old_cfg,
                new_cfg,
                Some("http://192.168.1.20:8000".to_string()),
                "networkmanager",
                |_iface, _cfg| async move { Ok(()) },
            )
            .await
            .expect("启动试运行失败");

        assert_eq!(op.status, OperationStatus::PendingConfirm);
        assert_eq!(op.interface_name, "eth0");

        // 查询 pending
        let pending = mgr.get_pending().await;
        assert!(pending.is_some());
        assert_eq!(pending.expect("pending 操作应存在").id, op.id);

        // 确认
        let confirm_res = mgr.confirm(&op.id).await.expect("确认试运行失败");
        assert_eq!(confirm_res.status, OperationStatus::Confirmed);
        assert!(confirm_res.confirmed_at.is_some());
    }

    #[tokio::test]
    async fn test_operation_manager_cancel_lifecycle() {
        let mgr = NetworkOperationManager::new();
        let old_cfg = sample_config("192.168.1.10");
        let new_cfg = sample_config("192.168.1.20");

        let rollback_called = Arc::new(tokio::sync::Mutex::new(false));
        let rollback_called_clone = Arc::clone(&rollback_called);

        let op = mgr
            .start_trial(
                "eth1",
                old_cfg,
                new_cfg,
                None,
                "systemd-networkd",
                move |_iface, _cfg| {
                    let called = Arc::clone(&rollback_called_clone);
                    async move {
                        *called.lock().await = true;
                        Ok(())
                    }
                },
            )
            .await
            .expect("启动试运行失败");

        let cancel_res = mgr
            .cancel(&op.id, |_iface, _cfg| async move { Ok(()) })
            .await
            .expect("取消试运行失败");

        assert_eq!(cancel_res.status, OperationStatus::Restored);
    }

    #[tokio::test]
    async fn test_operation_manager_abort_trial() {
        let mgr = NetworkOperationManager::new();
        let old_cfg = sample_config("192.168.1.10");
        let new_cfg = sample_config("192.168.1.20");

        let op = mgr
            .start_trial(
                "eth2",
                old_cfg,
                new_cfg,
                None,
                "networkmanager",
                |_iface, _cfg| async move { Ok(()) },
            )
            .await
            .expect("启动试运行失败");

        assert!(mgr.get_pending().await.is_some());

        // 模拟应用配置失败，中止并清理试运行事务
        mgr.abort_trial(&op.id).await;

        // 验证已清理，状态重置为 None
        assert!(mgr.get_pending().await.is_none());
    }
}
