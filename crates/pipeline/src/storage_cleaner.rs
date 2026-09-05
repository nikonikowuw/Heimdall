//! 统一存储池与 statvfs 加权原子淘汰引擎 (Atomic Eviction Engine)
//!
//! 1. 监控统一存储池（`var/data/evidence/`）物理磁盘使用率；
//! 2. 每 5 分钟或按需调用 POSIX `statvfs` 读取物理剩余空间百分比；
//! 3. 磁盘剩余 < 15% 时触发加权淘汰：**绝对优先批量淘汰普通抓拍记录（Captures），保全告警（Alarms）大图**；
//! 4. 严格恪守“图在案在，图销案销”，物理文件删除与数据库元数据清理同频原子联动。

use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::error::PipelineError;

/// 存储淘汰数据库存储接口
#[async_trait]
pub trait EvictionStore: Send + Sync {
    /// 查询最老的普通抓拍记录: (记录ID, 全景图相对路径, 特写图相对路径)
    async fn find_oldest_captures(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, PipelineError>;

    /// 按主键 ID 批量删除普通抓拍记录
    async fn delete_captures(&self, ids: &[i64]) -> Result<u64, PipelineError>;

    /// 查询最老的违规告警记录: (记录ID, 全景图相对路径, 特写图相对路径)
    async fn find_oldest_alarms(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, PipelineError>;

    /// 按主键 ID 批量删除违规告警记录
    async fn delete_alarms(&self, ids: &[i64]) -> Result<u64, PipelineError>;
}

/// 淘汰报告
#[derive(Debug, Clone)]
pub struct EvictionReport {
    pub free_ratio_before: f64,
    pub captures_deleted: u64,
    pub alarms_deleted: u64,
}

/// 存储清理器配置
#[derive(Debug, Clone)]
pub struct StorageCleanerConfig {
    pub evidence_dir: PathBuf,
    /// 最小磁盘剩余比例阈值（默认 0.15 即 15%）
    pub min_free_ratio: f64,
    /// 单批淘汰最大记录数（默认 100 条）
    pub batch_delete_size: u64,
}

impl Default for StorageCleanerConfig {
    fn default() -> Self {
        Self {
            evidence_dir: PathBuf::from("var/data/evidence"),
            min_free_ratio: 0.15,
            batch_delete_size: 100,
        }
    }
}

/// 存储淘汰清理器
#[derive(Debug)]
pub struct StorageCleaner {
    config: StorageCleanerConfig,
}

impl StorageCleaner {
    pub fn new(config: StorageCleanerConfig) -> Self {
        Self { config }
    }

    /// 检查磁盘水位并在空间不足 (< 15%) 时执行加权级联清理
    pub async fn clean_if_needed<S: EvictionStore>(
        &self,
        store: &S,
    ) -> Result<Option<EvictionReport>, PipelineError> {
        let free_ratio = get_disk_free_ratio(&self.config.evidence_dir)
            .map_err(|e| PipelineError::Snapshot(format!("检查磁盘剩余空间失败: {e}")))?;

        if free_ratio >= self.config.min_free_ratio {
            return Ok(None);
        }

        tracing::warn!(
            free_ratio = %format!("{:.2}%", free_ratio * 100.0),
            threshold = %format!("{:.2}%", self.config.min_free_ratio * 100.0),
            "磁盘剩余空间低于安全水位，触发加权原子存储淘汰机制"
        );

        let mut captures_deleted = 0;
        let mut alarms_deleted = 0;

        // 1. 绝对优先淘汰普通抓拍 (Captures)
        let oldest_caps = store
            .find_oldest_captures(self.config.batch_delete_size)
            .await?;

        if !oldest_caps.is_empty() {
            let cap_ids: Vec<i64> = oldest_caps.iter().map(|(id, _, _)| *id).collect();
            // 先执行数据库记录删除，确保数据库写事务成功，恪守“图在案在”一致性
            captures_deleted = store.delete_captures(&cap_ids).await?;
            self.delete_associated_files(&oldest_caps);
            tracing::info!(
                count = captures_deleted,
                "图在案在，图销案销：已级联清理普通抓拍切片与数据记录"
            );
        } else {
            // 2. 若普通抓拍已全空，极度匮乏时谨慎淘汰最老普通告警 (保全告警大图至最后阶段)
            let oldest_alarms = store
                .find_oldest_alarms(self.config.batch_delete_size)
                .await?;
            if !oldest_alarms.is_empty() {
                let alarm_ids: Vec<i64> = oldest_alarms.iter().map(|(id, _, _)| *id).collect();
                alarms_deleted = store.delete_alarms(&alarm_ids).await?;
                self.delete_associated_files(&oldest_alarms);
                tracing::warn!(
                    count = alarms_deleted,
                    "空间严重不足且无普通抓拍可删，已级联清理历史告警记录与图片"
                );
            }
        }

        Ok(Some(EvictionReport {
            free_ratio_before: free_ratio,
            captures_deleted,
            alarms_deleted,
        }))
    }

    /// 强制执行一批抓拍淘汰（用于自动化测试或手动运维调优）
    pub async fn force_clean_captures<S: EvictionStore>(
        &self,
        store: &S,
        limit: u64,
    ) -> Result<u64, PipelineError> {
        let oldest_caps = store.find_oldest_captures(limit).await?;
        if oldest_caps.is_empty() {
            return Ok(0);
        }

        let cap_ids: Vec<i64> = oldest_caps.iter().map(|(id, _, _)| *id).collect();
        let deleted = store.delete_captures(&cap_ids).await?;
        self.delete_associated_files(&oldest_caps);

        Ok(deleted)
    }

    /// 启动常驻后台定时巡检任务（默认 5 分钟巡检一次）
    pub fn start_periodic_worker<S: EvictionStore + 'static>(
        self: std::sync::Arc<Self>,
        store: std::sync::Arc<S>,
        interval: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                match self.clean_if_needed(store.as_ref()).await {
                    Ok(Some(report)) => {
                        tracing::info!(
                            before = %format!("{:.2}%", report.free_ratio_before * 100.0),
                            captures = report.captures_deleted,
                            alarms = report.alarms_deleted,
                            "定时存储水位巡检触发淘汰完成"
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "定时存储水位巡检异常");
                    }
                }
            }
        })
    }

    fn delete_associated_files(&self, records: &[(i64, String, String)]) {
        for (_id, full_rel, crop_rel) in records {
            self.remove_file_if_exists(full_rel);
            self.remove_file_if_exists(crop_rel);
        }
    }

    fn remove_file_if_exists(&self, rel_path: &str) {
        if rel_path.is_empty() {
            return;
        }
        let full_path = self.config.evidence_dir.join(rel_path);
        if full_path.is_file() {
            if let Err(e) = fs::remove_file(&full_path) {
                tracing::warn!(path = %full_path.display(), error = %e, "物理证据图片清理失败");
            }
        }
    }
}

/// 基于 POSIX statvfs 获取物理磁盘剩余空间百分比 (0.0 ~ 1.0)
pub fn get_disk_free_ratio(path: &Path) -> Result<f64, std::io::Error> {
    #[cfg(unix)]
    {
        // 保证路径存在，若不存在则向上寻找最近的祖先目录
        let mut probe_path = path.to_path_buf();
        while !probe_path.exists() {
            if let Some(parent) = probe_path.parent() {
                probe_path = parent.to_path_buf();
            } else {
                probe_path = PathBuf::from(".");
                break;
            }
        }

        let c_path = std::ffi::CString::new(probe_path.to_string_lossy().as_bytes())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

        // SAFETY: statvfs 结构体全零初始化为合法的安全内存
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: c_path 是合法的以 null 结尾的 C 字符串，stat 接收内核填充的磁盘统计
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if ret != 0 {
            return Err(std::io::Error::last_os_error());
        }

        if stat.f_blocks == 0 {
            return Ok(1.0);
        }

        let free_ratio = stat.f_bavail as f64 / stat.f_blocks as f64;
        Ok(free_ratio)
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(0.5) // 非 Unix 默认返回 50%
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use tokio::sync::RwLock;

    struct MockStore {
        captures: Arc<RwLock<Vec<(i64, String, String)>>>,
        alarms: Arc<RwLock<Vec<(i64, String, String)>>>,
        deleted_caps_count: AtomicU64,
    }

    #[async_trait]
    impl EvictionStore for MockStore {
        async fn find_oldest_captures(
            &self,
            limit: u64,
        ) -> Result<Vec<(i64, String, String)>, PipelineError> {
            let list = self.captures.read().await;
            Ok(list.iter().take(limit as usize).cloned().collect())
        }

        async fn delete_captures(&self, ids: &[i64]) -> Result<u64, PipelineError> {
            let mut list = self.captures.write().await;
            let initial = list.len();
            list.retain(|(id, _, _)| !ids.contains(id));
            let deleted = (initial - list.len()) as u64;
            self.deleted_caps_count
                .fetch_add(deleted, Ordering::Relaxed);
            Ok(deleted)
        }

        async fn find_oldest_alarms(
            &self,
            limit: u64,
        ) -> Result<Vec<(i64, String, String)>, PipelineError> {
            let list = self.alarms.read().await;
            Ok(list.iter().take(limit as usize).cloned().collect())
        }

        async fn delete_alarms(&self, ids: &[i64]) -> Result<u64, PipelineError> {
            let mut list = self.alarms.write().await;
            let initial = list.len();
            list.retain(|(id, _, _)| !ids.contains(id));
            Ok((initial - list.len()) as u64)
        }
    }

    #[tokio::test]
    async fn test_statvfs_and_atomic_cascade_eviction() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_evict_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&temp_dir).unwrap();

        // 验证真实 statvfs 能够读出宿主机磁盘空间比例
        let free_ratio = get_disk_free_ratio(&temp_dir).expect("statvfs should succeed");
        assert!(free_ratio > 0.0 && free_ratio <= 1.0);

        // 创建测试抓拍实体与磁盘文件
        let full_rel = "cam1/full.jpg".to_string();
        let crop_rel = "cam1/crop.jpg".to_string();
        let full_path = temp_dir.join(&full_rel);
        let crop_path = temp_dir.join(&crop_rel);
        fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        fs::write(&full_path, b"fake_jpeg").unwrap();
        fs::write(&crop_path, b"fake_crop").unwrap();

        assert!(full_path.is_file());
        assert!(crop_path.is_file());

        let store = MockStore {
            captures: Arc::new(RwLock::new(vec![(1, full_rel, crop_rel)])),
            alarms: Arc::new(RwLock::new(Vec::new())),
            deleted_caps_count: AtomicU64::new(0),
        };

        let cleaner = StorageCleaner::new(StorageCleanerConfig {
            evidence_dir: temp_dir.clone(),
            min_free_ratio: 0.99, // 设高以触发淘汰
            batch_delete_size: 10,
        });

        // 执行淘汰
        let report = cleaner
            .clean_if_needed(&store)
            .await
            .unwrap()
            .expect("should evict");
        assert_eq!(report.captures_deleted, 1);
        assert_eq!(report.alarms_deleted, 0);

        // 验证“图在案在，图销案销”：物理文件已被删除
        assert!(!full_path.exists(), "物理全景图必须被删除");
        assert!(!crop_path.exists(), "物理特写图必须被删除");
        assert_eq!(
            store.captures.read().await.len(),
            0,
            "数据库记录必须联动删除"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
