//! NPU 放置与生命周期统一管理门面 (PlacementManager)
//!
//! 提供高层统一接入入口，管理拓扑探测、决策求解与双层内存账本。

use std::sync::Arc;
use types::placement::{AffinityIntent, WirePlacementMetadata};

use super::inventory::DeviceInventory;
use super::ledger::{ExecutionReservation, LedgerConfig, PlacementLedger, ReservationId};
use super::solver::PlacementError;

/// 放置账本运行态监控快照
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlacementLedgerSnapshot {
    /// 当前拓扑代际
    pub topology_generation: u64,
    /// 活跃执行实例数
    pub active_reservations: usize,
    /// 活跃离线任务数
    pub active_offline_workers: u32,
    /// 各核心分配计数映射（键为 "device_id:mask"，值为活跃实例数）
    pub core_allocations: std::collections::HashMap<String, u32>,
    /// 物理权重缓存条目数
    pub active_weight_owners: usize,
}

/// 放置管理器统一门面
#[derive(Debug, Clone)]
pub struct PlacementManager {
    ledger: Arc<PlacementLedger>,
}

impl PlacementManager {
    /// 使用系统已探测的拓扑与默认配置初始化
    pub fn new(inventory: DeviceInventory) -> Self {
        Self::with_config(inventory, LedgerConfig::default())
    }

    /// 使用指定配置初始化
    pub fn with_config(inventory: DeviceInventory, config: LedgerConfig) -> Self {
        Self {
            ledger: Arc::new(PlacementLedger::new(inventory, config)),
        }
    }

    /// 获取底层账本强引用
    pub fn ledger(&self) -> &Arc<PlacementLedger> {
        &self.ledger
    }

    /// 原子预留放置资源
    pub fn reserve_placement(
        &self,
        instance_id: &str,
        group_id: &str,
        intent: &AffinityIntent,
        model_keys: &[String],
        package_generation: u64,
        is_offline: bool,
    ) -> Result<(ExecutionReservation, WirePlacementMetadata, bool), PlacementError> {
        self.ledger.reserve_placement(
            instance_id,
            group_id,
            intent,
            model_keys,
            package_generation,
            is_offline,
        )
    }

    /// 标记预留项就绪 (Ready)
    pub fn mark_ready(&self, id: ReservationId) -> Result<(), PlacementError> {
        self.ledger.mark_ready(id)
    }

    /// 标记预留项隔离 (Quarantined)
    pub fn mark_quarantined(&self, id: ReservationId) {
        self.ledger.mark_quarantined(id);
    }

    /// 释放预留项并回收配额
    pub fn release_placement(
        &self,
        id: ReservationId,
    ) -> Result<ExecutionReservation, PlacementError> {
        self.ledger.release_placement(id)
    }
}
