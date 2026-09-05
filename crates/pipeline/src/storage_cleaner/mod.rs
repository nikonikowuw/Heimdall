//! 统一存储池与工业级两阶段原子淘汰与自愈引擎 (Two-Phase Eviction & Self-Healing Engine)
//!
//! 彻底解决数据库事务与文件系统不在同一原子域的分布式一致性难题：
//! 1. **两阶段提交原子状态机**：
//!    - Phase 1: DB 事务将记录标记为 `deleting`（防重读与外溢）；
//!    - Phase 2: 同挂载点内原子重命名 (`rename`) 将待删证据移入 `.tombstone/` 隔离区；
//!    - Phase 3: DB 提交彻底物理删除；
//!    - Phase 4: 异步 Unlink 线程池批量释放磁盘空间。
//! 2. **崩溃自愈与墓碑扫描 (Startup Tombstone Sweep)**：
//!    - 启动时扫描 `.tombstone/` 目录，彻底回收由于掉电或进程崩溃遗留的待删孤儿文件。
//! 3. **全局孤儿与缺失对账引擎 (Orphan & Missing Reconciliation)**：
//!    - 比对物理磁盘文件与数据库活跃引用，识别无引用孤儿与物理丢失凭据。
//! 4. **路径安全强沙箱 (Path Security & Sandboxing)**：
//!    - 所有物理操作必须通过 `canonicalize` 严格校验在 `evidence_root` 之内，杜绝路径穿越。
//! 5. **失败重试与可观测性度量 (Retry Queue & Metrics)**：
//!    - 异步 Unlink 指数退避重试，维护完整的原子指标看板。

pub mod metrics;
pub mod path_security;
pub mod retry_queue;
pub mod tombstone;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::PipelineError;

pub use metrics::{EvictionMetrics, EvictionMetricsSnapshot};
pub use path_security::{relativize_safe_path, resolve_and_verify_evidence_path};
pub use retry_queue::UnlinkDispatcher;
pub use tombstone::{
    ensure_tombstone_dir, quarantine_file, sweep_tombstones_sync, TOMBSTONE_DIR_NAME,
};

/// 存储淘汰数据库抽象接口
#[async_trait]
pub trait EvictionStore: Send + Sync {
    /// 查询最老的普通抓拍记录: (记录ID, 全景图相对路径, 特写图相对路径)
    async fn find_oldest_captures(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, PipelineError>;

    /// 将抓拍记录预先标记为 deleting 状态 (两阶段提交第一阶段，可选实现)
    async fn mark_captures_deleting(&self, _ids: &[i64]) -> Result<u64, PipelineError> {
        Ok(0)
    }

    /// 按主键 ID 批量彻底删除普通抓拍记录 (两阶段提交第三阶段)
    async fn delete_captures(&self, ids: &[i64]) -> Result<u64, PipelineError>;

    /// 查询最老的违规告警记录: (记录ID, 全景图相对路径, 特写图相对路径)
    async fn find_oldest_alarms(
        &self,
        limit: u64,
    ) -> Result<Vec<(i64, String, String)>, PipelineError>;

    /// 将告警记录预先标记为 deleting 状态 (两阶段提交第一阶段，可选实现)
    async fn mark_alarms_deleting(&self, _ids: &[i64]) -> Result<u64, PipelineError> {
        Ok(0)
    }

    /// 按主键 ID 批量彻底删除违规告警记录 (两阶段提交第三阶段)
    async fn delete_alarms(&self, ids: &[i64]) -> Result<u64, PipelineError>;

    /// 查询当前数据库中所有活跃引用的图片相对路径集合 (用于全局孤儿扫描对账)
    async fn find_all_active_image_paths(&self) -> Result<HashSet<String>, PipelineError> {
        Ok(HashSet::new())
    }
}

/// 淘汰报告
#[derive(Debug, Clone)]
pub struct EvictionReport {
    pub free_ratio_before: f64,
    pub captures_deleted: u64,
    pub alarms_deleted: u64,
    pub quarantined_files: u64,
    pub missing_files: u64,
}

/// 全局对账扫描报告
#[derive(Debug, Clone)]
pub struct ReconciliationReport {
    pub total_scanned_files: u64,
    pub orphan_files_reclaimed: u64,
    pub missing_records_detected: u64,
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
            evidence_dir: PathBuf::from(crate::DEFAULT_EVIDENCE_DIR),
            min_free_ratio: 0.15,
            batch_delete_size: 100,
        }
    }
}

/// 工业级存储淘汰与自愈清理器
#[derive(Debug)]
pub struct StorageCleaner {
    config: StorageCleanerConfig,
    metrics: Arc<EvictionMetrics>,
    dispatcher: UnlinkDispatcher,
    _worker_handle: tokio::task::JoinHandle<()>,
}

impl StorageCleaner {
    pub fn new(config: StorageCleanerConfig) -> Self {
        let metrics = Arc::new(EvictionMetrics::new());
        let (dispatcher, worker_handle) = UnlinkDispatcher::start(Arc::clone(&metrics));

        Self {
            config,
            metrics,
            dispatcher,
            _worker_handle: worker_handle,
        }
    }

    /// 获取配置引用
    pub fn config(&self) -> &StorageCleanerConfig {
        &self.config
    }

    /// 获取指标原子引用
    pub fn metrics_ref(&self) -> &Arc<EvictionMetrics> {
        &self.metrics
    }

    /// 获取当前淘汰与对账指标快照
    pub fn metrics(&self) -> EvictionMetricsSnapshot {
        self.metrics.snapshot()
    }

    /// 启动时扫描并清空墓碑隔离区 (.tombstone/)，实现掉电与崩溃后的自动空间回收
    pub fn sweep_tombstones(&self) -> Result<u64, PipelineError> {
        let count = sweep_tombstones_sync(&self.config.evidence_dir)?;
        if count > 0 {
            self.metrics
                .tombstone_reclaimed_total
                .fetch_add(count, Ordering::Relaxed);
            tracing::info!(
                count,
                tombstone_dir = %self.config.evidence_dir.join(TOMBSTONE_DIR_NAME).display(),
                "系统自愈：启动时已自动扫描并回收历史遗留墓碑文件"
            );
        }
        Ok(count)
    }

    /// 等待未决的异步 Unlink 任务处理完毕 (主要用于测试或优雅退出)
    pub async fn flush_pending_unlinks(&self) {
        self.dispatcher.flush_wait().await;
    }

    /// 检查磁盘水位并在空间不足 (< 15%) 时执行两阶段原子加权级联清理
    pub async fn clean_if_needed<S: EvictionStore>(
        &self,
        store: &S,
    ) -> Result<Option<EvictionReport>, PipelineError> {
        self.metrics
            .clean_cycles_total
            .fetch_add(1, Ordering::Relaxed);

        let free_ratio = get_disk_free_ratio(&self.config.evidence_dir)
            .map_err(|e| PipelineError::Snapshot(format!("检查磁盘剩余空间失败: {e}")))?;

        if free_ratio >= self.config.min_free_ratio {
            return Ok(None);
        }

        tracing::warn!(
            free_ratio = %format!("{:.2}%", free_ratio * 100.0),
            threshold = %format!("{:.2}%", self.config.min_free_ratio * 100.0),
            "磁盘剩余空间低于安全水位，触发工业级两阶段原子加权淘汰机制"
        );

        let mut captures_deleted = 0;
        let mut alarms_deleted = 0;
        let mut quarantined_files = 0;
        let mut missing_files = 0;

        // 1. 绝对优先淘汰普通抓拍 (Captures)
        let oldest_caps = store
            .find_oldest_captures(self.config.batch_delete_size)
            .await?;

        if !oldest_caps.is_empty() {
            let cap_ids: Vec<i64> = oldest_caps.iter().map(|(id, _, _)| *id).collect();

            // Phase 1: DB 预标记
            let _ = store.mark_captures_deleting(&cap_ids).await?;

            // Phase 2: 将物理文件原子重命名至墓碑隔离区
            let (quarantined_paths, missing) = self.quarantine_record_files(&oldest_caps);
            quarantined_files = quarantined_paths.len() as u64;
            missing_files = missing;

            // Phase 3: DB 提交彻底物理删除
            captures_deleted = store.delete_captures(&cap_ids).await?;

            // Phase 4: 异步将墓碑隔离区文件分发至 Unlink 队列
            self.dispatcher.dispatch_batch(quarantined_paths).await;

            self.metrics
                .captures_evicted_total
                .fetch_add(captures_deleted, Ordering::Relaxed);

            tracing::info!(
                count = captures_deleted,
                quarantined = quarantined_files,
                missing = missing_files,
                "图在案在，图销案销：已完成普通抓拍两阶段原子淘汰"
            );
        } else {
            // 2. 若普通抓拍已全空，极度匮乏时谨慎淘汰最老普通告警 (保全告警大图至最后阶段)
            let oldest_alarms = store
                .find_oldest_alarms(self.config.batch_delete_size)
                .await?;

            if !oldest_alarms.is_empty() {
                let alarm_ids: Vec<i64> = oldest_alarms.iter().map(|(id, _, _)| *id).collect();

                // Phase 1: DB 预标记
                let _ = store.mark_alarms_deleting(&alarm_ids).await?;

                // Phase 2: 将物理文件原子重命名至墓碑隔离区
                let (quarantined_paths, missing) = self.quarantine_record_files(&oldest_alarms);
                quarantined_files = quarantined_paths.len() as u64;
                missing_files = missing;

                // Phase 3: DB 提交彻底物理删除
                alarms_deleted = store.delete_alarms(&alarm_ids).await?;

                // Phase 4: 异步 Unlink
                self.dispatcher.dispatch_batch(quarantined_paths).await;

                self.metrics
                    .alarms_evicted_total
                    .fetch_add(alarms_deleted, Ordering::Relaxed);

                tracing::warn!(
                    count = alarms_deleted,
                    quarantined = quarantined_files,
                    missing = missing_files,
                    "空间严重不足且无普通抓拍可删，已执行历史告警两阶段原子淘汰"
                );
            }
        }

        Ok(Some(EvictionReport {
            free_ratio_before: free_ratio,
            captures_deleted,
            alarms_deleted,
            quarantined_files,
            missing_files,
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
        let _ = store.mark_captures_deleting(&cap_ids).await?;
        let (quarantined_paths, _) = self.quarantine_record_files(&oldest_caps);
        let deleted = store.delete_captures(&cap_ids).await?;
        self.dispatcher.dispatch_batch(quarantined_paths).await;

        self.metrics
            .captures_evicted_total
            .fetch_add(deleted, Ordering::Relaxed);

        Ok(deleted)
    }

    /// 将记录集合关联的文件批量原子隔离至墓碑目录
    fn quarantine_record_files(&self, records: &[(i64, String, String)]) -> (Vec<PathBuf>, u64) {
        let mut quarantined_paths = Vec::new();
        let mut missing_count = 0;

        for (_id, full_rel, crop_rel) in records {
            for rel in [full_rel, crop_rel] {
                if rel.is_empty() {
                    continue;
                }
                match quarantine_file(&self.config.evidence_dir, rel) {
                    Ok(Some(tombstone_path)) => {
                        quarantined_paths.push(tombstone_path);
                    }
                    Ok(None) => {
                        // 物理文件不存在，记录丢失凭据
                        missing_count += 1;
                        self.metrics
                            .missing_files_detected_total
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        tracing::error!(
                            rel_path = %rel,
                            error = %e,
                            "物理文件移入墓碑隔离区遇到异常，跳过该文件继续推进"
                        );
                    }
                }
            }
        }
        (quarantined_paths, missing_count)
    }

    /// 全局孤儿文件与缺失凭据对账自愈扫描 (Reconciliation Loop)
    ///
    /// - 扫描物理磁盘与数据库中的活跃记录；
    /// - 发现物理磁盘存在但数据库中无引用的孤儿文件，自动隔离并异步清理；
    /// - 发现数据库有记录但物理磁盘缺失的记录，累计指标并记录告警日志。
    pub async fn reconcile_orphans<S: EvictionStore>(
        &self,
        store: &S,
    ) -> Result<ReconciliationReport, PipelineError> {
        let active_paths = store.find_all_active_image_paths().await?;
        let root = &self.config.evidence_dir;
        if !root.exists() {
            return Ok(ReconciliationReport {
                total_scanned_files: 0,
                orphan_files_reclaimed: 0,
                missing_records_detected: 0,
            });
        }

        let mut scanned = 0;
        let mut orphan_paths = Vec::new();

        // 递归扫描证据根目录
        self.collect_disk_files(root, root, &mut scanned, &mut orphan_paths, &active_paths)?;

        let orphan_count = orphan_paths.len() as u64;
        if orphan_count > 0 {
            tracing::warn!(
                count = orphan_count,
                "对账扫描发现磁盘存在孤儿证据文件，立即移入墓碑隔离区回收"
            );
            for rel_path in orphan_paths {
                self.metrics
                    .orphan_files_detected_total
                    .fetch_add(1, Ordering::Relaxed);
                if let Ok(Some(tombstone_p)) = quarantine_file(root, &rel_path) {
                    self.dispatcher.dispatch(tombstone_p).await;
                }
            }
        }

        // 检查数据库记录缺失
        let mut missing_records = 0;
        for db_rel in &active_paths {
            if let Ok(safe_p) = resolve_and_verify_evidence_path(root, db_rel) {
                if !safe_p.is_file() {
                    missing_records += 1;
                    self.metrics
                        .missing_files_detected_total
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        rel_path = %db_rel,
                        "对账发现数据库活跃记录在物理磁盘上已丢失 (Ghost Record)"
                    );
                }
            }
        }

        Ok(ReconciliationReport {
            total_scanned_files: scanned,
            orphan_files_reclaimed: orphan_count,
            missing_records_detected: missing_records,
        })
    }

    fn collect_disk_files(
        &self,
        root: &Path,
        current_dir: &Path,
        scanned: &mut u64,
        orphan_paths: &mut Vec<String>,
        active_paths: &HashSet<String>,
    ) -> Result<(), PipelineError> {
        let entries = match fs::read_dir(current_dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // 跳过隐藏文件及 .tombstone 墓碑区
            if name_str.starts_with('.') || name_str == TOMBSTONE_DIR_NAME {
                continue;
            }

            if path.is_dir() {
                self.collect_disk_files(root, &path, scanned, orphan_paths, active_paths)?;
            } else if path.is_file() {
                *scanned += 1;
                if let Ok(rel) = relativize_safe_path(root, &path) {
                    if !active_paths.contains(&rel) {
                        orphan_paths.push(rel);
                    }
                }
            }
        }
        Ok(())
    }

    /// 启动常驻后台定时巡检任务（默认 5 分钟巡检一次）
    pub fn start_periodic_worker<S: EvictionStore + 'static>(
        self: std::sync::Arc<Self>,
        store: std::sync::Arc<S>,
        interval: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // 启动时优先执行一次墓碑扫描自愈
            let _ = self.sweep_tombstones();

            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                match self.clean_if_needed(store.as_ref()).await {
                    Ok(Some(report)) => {
                        tracing::info!(
                            before = %format!("{:.2}%", report.free_ratio_before * 100.0),
                            captures = report.captures_deleted,
                            alarms = report.alarms_deleted,
                            quarantined = report.quarantined_files,
                            missing = report.missing_files,
                            "定时存储水位巡检触发两阶段淘汰完成"
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
        deleted_alarms_count: AtomicU64,
        marked_deleting_count: AtomicU64,
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

        async fn mark_captures_deleting(&self, ids: &[i64]) -> Result<u64, PipelineError> {
            self.marked_deleting_count
                .fetch_add(ids.len() as u64, Ordering::Relaxed);
            Ok(ids.len() as u64)
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

        async fn mark_alarms_deleting(&self, ids: &[i64]) -> Result<u64, PipelineError> {
            self.marked_deleting_count
                .fetch_add(ids.len() as u64, Ordering::Relaxed);
            Ok(ids.len() as u64)
        }

        async fn delete_alarms(&self, ids: &[i64]) -> Result<u64, PipelineError> {
            let mut list = self.alarms.write().await;
            let initial = list.len();
            list.retain(|(id, _, _)| !ids.contains(id));
            let deleted = (initial - list.len()) as u64;
            self.deleted_alarms_count
                .fetch_add(deleted, Ordering::Relaxed);
            Ok(deleted)
        }

        async fn find_all_active_image_paths(&self) -> Result<HashSet<String>, PipelineError> {
            let mut set = HashSet::new();
            for (_, f, c) in self.captures.read().await.iter() {
                if !f.is_empty() {
                    set.insert(f.clone());
                }
                if !c.is_empty() {
                    set.insert(c.clone());
                }
            }
            for (_, f, c) in self.alarms.read().await.iter() {
                if !f.is_empty() {
                    set.insert(f.clone());
                }
                if !c.is_empty() {
                    set.insert(c.clone());
                }
            }
            Ok(set)
        }
    }

    #[tokio::test]
    async fn test_two_phase_atomic_eviction_and_metrics() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_eviction_twophase_{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        // 创建测试证据文件
        let cam_dir = temp_dir.join("cam_01");
        fs::create_dir_all(&cam_dir).unwrap();

        let full_img = cam_dir.join("img_01.jpg");
        let crop_img = cam_dir.join("crop_01.jpg");
        fs::write(&full_img, b"fake_jpeg_content").unwrap();
        fs::write(&crop_img, b"fake_crop_content").unwrap();

        let store = MockStore {
            captures: Arc::new(RwLock::new(vec![(
                1,
                "cam_01/img_01.jpg".to_string(),
                "cam_01/crop_01.jpg".to_string(),
            )])),
            alarms: Arc::new(RwLock::new(Vec::new())),
            deleted_caps_count: AtomicU64::new(0),
            deleted_alarms_count: AtomicU64::new(0),
            marked_deleting_count: AtomicU64::new(0),
        };

        let cleaner = StorageCleaner::new(StorageCleanerConfig {
            evidence_dir: temp_dir.clone(),
            min_free_ratio: 0.99, // 强制触发
            batch_delete_size: 10,
        });

        let report = cleaner
            .clean_if_needed(&store)
            .await
            .unwrap()
            .expect("应触发淘汰");

        assert_eq!(report.captures_deleted, 1);
        assert_eq!(report.quarantined_files, 2);
        assert_eq!(store.deleted_caps_count.load(Ordering::Relaxed), 1);
        assert_eq!(store.marked_deleting_count.load(Ordering::Relaxed), 1);

        // 原路径应立即不存在（已原子 move 到 tombstone）
        assert!(!full_img.exists());
        assert!(!crop_img.exists());

        // 等待异步 Unlink 完成
        cleaner.flush_pending_unlinks().await;

        let metrics = cleaner.metrics();
        assert_eq!(metrics.captures_evicted_total, 1);
        assert!(metrics.tombstone_reclaimed_total >= 2);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_startup_tombstone_sweep_and_orphan_reconciliation() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_sweep_orphan_{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

        // 1. 模拟崩溃遗留的墓碑文件
        let tombstone_dir = temp_dir.join(TOMBSTONE_DIR_NAME);
        fs::create_dir_all(&tombstone_dir).unwrap();
        let ghost_file = tombstone_dir.join("100000_abc_legacy.jpg");
        fs::write(&ghost_file, b"ghost").unwrap();

        // 2. 模拟孤儿文件（磁盘上有，DB 中无记录）
        let cam_dir = temp_dir.join("cam_99");
        fs::create_dir_all(&cam_dir).unwrap();
        let orphan_file = cam_dir.join("orphan.jpg");
        fs::write(&orphan_file, b"orphan").unwrap();

        let store = MockStore {
            captures: Arc::new(RwLock::new(Vec::new())),
            alarms: Arc::new(RwLock::new(Vec::new())),
            deleted_caps_count: AtomicU64::new(0),
            deleted_alarms_count: AtomicU64::new(0),
            marked_deleting_count: AtomicU64::new(0),
        };

        let cleaner = StorageCleaner::new(StorageCleanerConfig {
            evidence_dir: temp_dir.clone(),
            min_free_ratio: 0.1,
            batch_delete_size: 10,
        });

        // 启动自愈扫描 Tombstone
        let swept = cleaner.sweep_tombstones().unwrap();
        assert_eq!(swept, 1);
        assert!(!ghost_file.exists());

        // 对账扫描孤儿
        let recon = cleaner.reconcile_orphans(&store).await.unwrap();
        assert_eq!(recon.orphan_files_reclaimed, 1);
        assert!(!orphan_file.exists());

        cleaner.flush_pending_unlinks().await;

        let metrics = cleaner.metrics();
        assert!(metrics.orphan_files_detected_total >= 1);
        assert!(metrics.tombstone_reclaimed_total >= 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
