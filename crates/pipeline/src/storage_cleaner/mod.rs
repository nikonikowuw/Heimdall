//! 统一存储池与工业级自适应两阶段原子淘汰与多级防护引擎 (Adaptive Multi-tier Eviction Engine)
//!
//! 彻底解决“单次仅删 100 条无法回到安全水位”、“缺乏回滞防抖”、“缺乏 Inode 耗尽感知”与“写盘硬熔断缺失”等问题：
//! 1. **回滞防抖与安全低水位 (Hysteresis)**：
//!    - 高水位触发淘汰 (`trigger_free_ratio = 15%`)；
//!    - 连续自适应排空循环 (`Continuous Drain Loop`) 直至回蓄至目标安全低水位 (`target_free_ratio = 25%`)；
//! 2. **四级健康状态机与紧急加速 (Multi-tier Health & Emergency Mode)**：
//!    - `Normal` -> `Evicting` -> `Emergency` (< 8% 空间时加倍批次、冻结普通抓拍) -> `Critical` (< 5% 全域写熔断)；
//! 3. **Inode 节点与挂载只读全维度感知 (Inode & ROFS Detection)**：
//!    - 单次 `statvfs` 同时提取字节容量、可用 Inode 节点数及只读标志 `ST_RDONLY`；
//! 4. **两阶段提交原子状态机与崩溃自愈**：
//!    - 预标记 `deleting` -> 原子 `rename` 移入 `.tombstone/` -> 提交 DB 删除 -> 异步批量 Unlink；
//!    - 启动时自动扫描 `.tombstone/` 回收残留孤儿；
//! 5. **自适应巡检频率**：
//!    - 空间充足时长周期 (5 min)，淘汰中或处于紧急水位时自适应提速 (30s) 紧密跟踪。

pub mod fs_stat;
pub mod guard;
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

pub use fs_stat::{
    detect_emmc_health, get_sqlite_wal_size, stat_fs, EmmcHealthInfo, FsStorageStat,
};
pub use guard::{
    StorageCircuitBreaker, StorageDecision, StorageHealthLevel, StorageWatermarkThresholds,
};
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
    pub free_ratio_after: f64,
    pub captures_deleted: u64,
    pub alarms_deleted: u64,
    pub quarantined_files: u64,
    pub missing_files: u64,
    pub drain_iterations: u32,
}

/// 全局对账扫描报告
#[derive(Debug, Clone)]
pub struct ReconciliationReport {
    pub total_scanned_files: u64,
    pub orphan_files_reclaimed: u64,
    pub missing_records_detected: u64,
}

/// 工业级存储清理器配置
#[derive(Debug, Clone)]
pub struct StorageCleanerConfig {
    pub evidence_dir: PathBuf,
    /// 触发淘汰的高水位阈值（默认 0.15 即 15%）
    pub min_free_ratio: f64,
    /// 单批淘汰最大记录数（默认 100 条）
    pub batch_delete_size: u64,
    /// 停止淘汰的目标安全低水位（回滞 Hysteresis，默认 0.25 即 25%）
    pub target_free_ratio: f64,
    /// 紧急严重水位阈值（默认 0.08 即 8%）
    pub emergency_free_ratio: f64,
    /// 临界写保护硬熔断阈值（默认 0.05 即 5%）
    pub critical_free_ratio: f64,
    /// Inode 触发淘汰阈值（默认 0.10 即 10%）
    pub min_inode_free_ratio: f64,
    /// Inode 临界硬熔断阈值（默认 0.02 即 2%）
    pub critical_inode_free_ratio: f64,
    /// 单次清理巡检允许执行的最大连续淘汰子轮次（防止外部文件占满时死循环，默认 50 轮）
    pub max_drain_iterations: u32,
}

impl StorageCleanerConfig {
    /// 快捷设置统一最小触发水位 (自动推导 target_free_ratio 回滞水位)
    pub fn with_min_free_ratio(mut self, min_ratio: f64) -> Self {
        self.min_free_ratio = min_ratio;
        self.target_free_ratio = (min_ratio + 0.10).min(0.999);
        self
    }

    /// 提取结构化的存储水位与断路阈值
    pub fn thresholds(&self) -> StorageWatermarkThresholds {
        StorageWatermarkThresholds {
            trigger_free_ratio: self.min_free_ratio,
            emergency_free_ratio: self.emergency_free_ratio,
            critical_free_ratio: self.critical_free_ratio,
            min_inode_free_ratio: self.min_inode_free_ratio,
            critical_inode_free_ratio: self.critical_inode_free_ratio,
        }
    }
}

impl Default for StorageCleanerConfig {
    fn default() -> Self {
        Self {
            evidence_dir: PathBuf::from(crate::DEFAULT_EVIDENCE_DIR),
            min_free_ratio: 0.15,
            batch_delete_size: 100,
            target_free_ratio: 0.25,
            emergency_free_ratio: 0.08,
            critical_free_ratio: 0.05,
            min_inode_free_ratio: 0.10,
            critical_inode_free_ratio: 0.02,
            max_drain_iterations: 50,
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

    /// 获取当前文件系统的物理状态
    pub fn current_fs_stat(&self) -> Result<FsStorageStat, std::io::Error> {
        stat_fs(&self.config.evidence_dir)
    }

    /// 评估当前存储健康等级与写入放行决策
    pub fn evaluate_storage_health(&self) -> Result<StorageDecision, std::io::Error> {
        let stat = self.current_fs_stat()?;
        Ok(StorageCircuitBreaker::evaluate(
            &stat,
            &self.config.thresholds(),
        ))
    }

    /// 检查当前文件系统是否低于回蓄安全低水位（空间或 Inode）
    #[inline]
    fn is_below_target(&self, stat: &FsStorageStat, target_ratio: f64) -> bool {
        stat.free_ratio < target_ratio
            || stat.inode_free_ratio < (self.config.min_inode_free_ratio * 1.5)
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

    /// 检查磁盘与 Inode 水位，在空间不足时执行工业级回滞连续淘汰循环 (Continuous Drain Loop)
    pub async fn clean_if_needed<S: EvictionStore>(
        &self,
        store: &S,
    ) -> Result<Option<EvictionReport>, PipelineError> {
        self.metrics
            .clean_cycles_total
            .fetch_add(1, Ordering::Relaxed);

        let mut stat = stat_fs(&self.config.evidence_dir)
            .map_err(|e| PipelineError::Snapshot(format!("读取文件系统状态失败: {e}")))?;

        let target_ratio = self
            .config
            .target_free_ratio
            .max(self.config.min_free_ratio);
        let decision = StorageCircuitBreaker::evaluate(&stat, &self.config.thresholds());

        if !decision.should_evict {
            return Ok(None);
        }

        let free_ratio_before = stat.free_ratio;

        tracing::warn!(
            health_level = ?decision.level,
            free_ratio = %format!("{:.2}%", stat.free_ratio * 100.0),
            inode_free = %format!("{:.2}%", stat.inode_free_ratio * 100.0),
            target_free = %format!("{:.2}%", target_ratio * 100.0),
            available = %stat.format_available_bytes(),
            "触发工业级回滞自适应连续淘汰循环 (Continuous Drain Loop)"
        );

        let mut total_captures_deleted = 0;
        let mut total_alarms_deleted = 0;
        let mut total_quarantined_files = 0;
        let mut total_missing_files = 0;
        let mut drain_iterations = 0;

        // 【核心连续排空循环】
        // 持续批量淘汰，直到物理剩余空间和 Inode 均回蓄至安全目标低水位 (target_free_ratio)，
        // 或者无数据可删，或者达到单轮迭代保护上限 (max_drain_iterations)
        while self.is_below_target(&stat, target_ratio)
            && drain_iterations < self.config.max_drain_iterations
        {
            drain_iterations += 1;
            self.metrics
                .drain_iterations_total
                .fetch_add(1, Ordering::Relaxed);

            let is_emergency = stat.free_ratio < self.config.emergency_free_ratio
                || stat.inode_free_ratio < (self.config.critical_inode_free_ratio * 2.0);

            if is_emergency {
                self.metrics
                    .emergency_evictions_total
                    .fetch_add(1, Ordering::Relaxed);
            }

            // 紧急状态下动态加倍批次大小以加速排空
            let current_batch_size = if is_emergency {
                self.config.batch_delete_size.saturating_mul(2)
            } else {
                self.config.batch_delete_size
            };

            // 1. 优先淘汰抓拍记录 (Captures)
            let oldest_caps = store.find_oldest_captures(current_batch_size).await?;

            if !oldest_caps.is_empty() {
                let cap_ids: Vec<i64> = oldest_caps.iter().map(|(id, _, _)| *id).collect();

                // Phase 1: DB 预标记
                let _ = store.mark_captures_deleting(&cap_ids).await?;

                // Phase 2: 将物理文件原子重命名至墓碑隔离区
                let (quarantined_paths, missing) = self.quarantine_record_files(&oldest_caps);
                total_quarantined_files += quarantined_paths.len() as u64;
                total_missing_files += missing;

                // Phase 3: DB 提交彻底物理删除
                let deleted = store.delete_captures(&cap_ids).await?;
                total_captures_deleted += deleted;

                // Phase 4: 异步 Unlink 批量释放物理磁盘
                self.dispatcher.dispatch_batch(quarantined_paths).await;
            } else if is_emergency {
                // 2. 普通抓拍已空，且处于紧急严重水位 (< 8%) 时，才谨慎淘汰历史告警 (Alarms)
                let oldest_alarms = store.find_oldest_alarms(current_batch_size).await?;

                if !oldest_alarms.is_empty() {
                    let alarm_ids: Vec<i64> = oldest_alarms.iter().map(|(id, _, _)| *id).collect();

                    // Phase 1: DB 预标记
                    let _ = store.mark_alarms_deleting(&alarm_ids).await?;

                    // Phase 2: 隔离
                    let (quarantined_paths, missing) = self.quarantine_record_files(&oldest_alarms);
                    total_quarantined_files += quarantined_paths.len() as u64;
                    total_missing_files += missing;

                    // Phase 3: DB 提交删除
                    let deleted = store.delete_alarms(&alarm_ids).await?;
                    total_alarms_deleted += deleted;

                    // Phase 4: 异步 Unlink
                    self.dispatcher.dispatch_batch(quarantined_paths).await;
                } else {
                    tracing::warn!(
                        free_ratio = %format!("{:.2}%", stat.free_ratio * 100.0),
                        "数据库中抓拍与告警记录均已排空，自适应退出排空循环"
                    );
                    break;
                }
            } else {
                // 普通抓拍已全部淘汰完毕，当前未触碰紧急生死线，保全核心违规告警大图！
                tracing::info!(
                    free_ratio = %format!("{:.2}%", stat.free_ratio * 100.0),
                    emergency_threshold = %format!("{:.2}%", self.config.emergency_free_ratio * 100.0),
                    "普通抓拍已排空；当前未触碰紧急红线，保全核心告警凭据，退出排空循环"
                );
                break;
            }

            // 同步等待当前批次物理 unlink 完毕，以获得准确的文件系统反馈
            self.flush_pending_unlinks().await;

            // 重新刷新采样物理状态
            if let Ok(new_stat) = stat_fs(&self.config.evidence_dir) {
                stat = new_stat;
            } else {
                break;
            }
        }

        self.metrics
            .captures_evicted_total
            .fetch_add(total_captures_deleted, Ordering::Relaxed);
        self.metrics
            .alarms_evicted_total
            .fetch_add(total_alarms_deleted, Ordering::Relaxed);

        let free_ratio_after = stat.free_ratio;

        tracing::info!(
            iterations = drain_iterations,
            before = %format!("{:.2}%", free_ratio_before * 100.0),
            after = %format!("{:.2}%", free_ratio_after * 100.0),
            captures = total_captures_deleted,
            alarms = total_alarms_deleted,
            quarantined = total_quarantined_files,
            missing = total_missing_files,
            "连续回滞排空循环执行完毕"
        );

        Ok(Some(EvictionReport {
            free_ratio_before,
            free_ratio_after,
            captures_deleted: total_captures_deleted,
            alarms_deleted: total_alarms_deleted,
            quarantined_files: total_quarantined_files,
            missing_files: total_missing_files,
            drain_iterations,
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

    /// 启动常驻后台自适应定时巡检任务
    ///
    /// - 空间充足时休眠 default_interval (默认 5 分钟)；
    /// - 处于淘汰中或紧急水位时，按健康评估建议自适应缩短周期紧密跟踪。
    pub fn start_periodic_worker<S: EvictionStore + 'static>(
        self: std::sync::Arc<Self>,
        store: std::sync::Arc<S>,
        default_interval: std::time::Duration,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let _ = self.sweep_tombstones();

            let mut current_interval = default_interval;
            loop {
                tokio::time::sleep(current_interval).await;
                match self.clean_if_needed(store.as_ref()).await {
                    Ok(Some(report)) => {
                        tracing::info!(
                            iterations = report.drain_iterations,
                            before = %format!("{:.2}%", report.free_ratio_before * 100.0),
                            after = %format!("{:.2}%", report.free_ratio_after * 100.0),
                            captures = report.captures_deleted,
                            alarms = report.alarms_deleted,
                            quarantined = report.quarantined_files,
                            missing = report.missing_files,
                            "定时自适应存储水位巡检淘汰完成"
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "定时存储水位巡检异常");
                    }
                }

                // 根据实时健康状态动态调整下次巡检间隔
                current_interval = match self.evaluate_storage_health() {
                    Ok(decision) if decision.level == StorageHealthLevel::Normal => {
                        default_interval
                    }
                    Ok(decision) => decision.suggested_poll_interval,
                    Err(_) => std::time::Duration::from_secs(30),
                };
            }
        })
    }
}

/// 基于 POSIX statvfs 获取物理磁盘剩余空间百分比 (向后兼容助手函数)
pub fn get_disk_free_ratio(path: &Path) -> Result<f64, std::io::Error> {
    stat_fs(path).map(|s| s.free_ratio)
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
    async fn test_continuous_drain_loop_multi_batch() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_drain_loop_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&temp_dir).unwrap();

        // 构造 5 个抓拍记录
        let mut caps = Vec::new();
        for i in 1..=5 {
            let cam_dir = temp_dir.join(format!("cam_{i}"));
            fs::create_dir_all(&cam_dir).unwrap();
            let full_p = cam_dir.join(format!("img_{i}.jpg"));
            let crop_p = cam_dir.join(format!("crop_{i}.jpg"));
            fs::write(&full_p, b"data").unwrap();
            fs::write(&crop_p, b"data").unwrap();
            caps.push((
                i as i64,
                format!("cam_{i}/img_{i}.jpg"),
                format!("cam_{i}/crop_{i}.jpg"),
            ));
        }

        let store = MockStore {
            captures: Arc::new(RwLock::new(caps)),
            alarms: Arc::new(RwLock::new(Vec::new())),
            deleted_caps_count: AtomicU64::new(0),
            deleted_alarms_count: AtomicU64::new(0),
            marked_deleting_count: AtomicU64::new(0),
        };

        // 设置单批仅删 2 条，但 target_free_ratio 设为 0.99 强制持续排空！
        let cleaner = StorageCleaner::new(StorageCleanerConfig {
            evidence_dir: temp_dir.clone(),
            min_free_ratio: 0.99,
            target_free_ratio: 0.999,
            emergency_free_ratio: 0.08,
            critical_free_ratio: 0.05,
            min_inode_free_ratio: 0.10,
            critical_inode_free_ratio: 0.02,
            batch_delete_size: 2, // 批次为 2
            max_drain_iterations: 10,
        });

        let report = cleaner
            .clean_if_needed(&store)
            .await
            .unwrap()
            .expect("应触发连续排空");

        // 5 条记录以每批 2 条连续排空，总共应排空 3 轮，删完所有 5 条！
        assert_eq!(report.captures_deleted, 5);
        assert!(report.drain_iterations >= 3);
        assert_eq!(store.captures.read().await.len(), 0);

        cleaner.flush_pending_unlinks().await;
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_two_phase_atomic_eviction_and_metrics() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_eviction_twophase_{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&temp_dir).unwrap();

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

        let mut cfg = StorageCleanerConfig::default().with_min_free_ratio(0.99);
        cfg.evidence_dir = temp_dir.clone();
        let cleaner = StorageCleaner::new(cfg);

        let report = cleaner
            .clean_if_needed(&store)
            .await
            .unwrap()
            .expect("应触发淘汰");

        assert_eq!(report.captures_deleted, 1);
        assert_eq!(report.quarantined_files, 2);
        assert_eq!(store.deleted_caps_count.load(Ordering::Relaxed), 1);
        assert_eq!(store.marked_deleting_count.load(Ordering::Relaxed), 1);

        assert!(!full_img.exists());
        assert!(!crop_img.exists());

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

        let tombstone_dir = temp_dir.join(TOMBSTONE_DIR_NAME);
        fs::create_dir_all(&tombstone_dir).unwrap();
        let ghost_file = tombstone_dir.join("100000_abc_legacy.jpg");
        fs::write(&ghost_file, b"ghost").unwrap();

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
            target_free_ratio: 0.2,
            emergency_free_ratio: 0.05,
            critical_free_ratio: 0.02,
            min_inode_free_ratio: 0.1,
            critical_inode_free_ratio: 0.02,
            batch_delete_size: 10,
            max_drain_iterations: 10,
        });

        let swept = cleaner.sweep_tombstones().unwrap();
        assert_eq!(swept, 1);
        assert!(!ghost_file.exists());

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
