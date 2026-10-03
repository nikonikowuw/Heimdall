//! NPU 双层账本与状态机 (ExecutionLedger & WeightLedger)
//!
//! 在短持有锁内执行原子资源预留与释放，提供单航班权重加载支持与熔断保护。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use types::placement::{AffinityIntent, WirePlacementMetadata, WireWeightBinding};

use super::inventory::DeviceInventory;
use super::solver::{PlacementDecision, PlacementError, PlacementSolver};
use crate::worker::quarantined_workers_count;

static RESERVATION_SEQ: AtomicU64 = AtomicU64::new(100);

/// 资源预留唯一数字 ID
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub u64);

/// 物理权重缓存键
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeightKey {
    /// 模型规范键（如 "yolov8n", "scrfd"）
    pub model_key: String,
    /// 绑定的物理设备 ID（如 "rknn-npu0"）
    pub device_id: String,
    /// 算法包制品代际号
    pub package_generation: u64,
}

/// 执行实例生命周期状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    /// 额度已预留，工作线程尚未启动
    Reserved,
    /// 工作线程正在启动并初始化 backend
    Initializing,
    /// 运行就绪，可处理推理
    Ready,
    /// 正在注销停机
    Draining,
    /// 异常隔离挂起
    Quarantined,
    /// 已彻底释放
    Released,
}

/// 权重所有权生命周期状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightState {
    /// 正在预留/初始化
    Initializing,
    /// 热态就绪中
    Ready,
    /// 冷却退火中（无活跃实例使用）
    Cooling,
    /// 已释放
    Released,
}

/// 单个执行实例的资源预留项
#[derive(Debug, Clone)]
pub struct ExecutionReservation {
    pub id: ReservationId,
    pub reservation_string: String,
    pub instance_id: String,
    pub group_id: String,
    pub generation: u64,
    pub topology_generation: u64,
    pub decision: PlacementDecision,
    pub state: ExecutionState,
    pub is_offline: bool,
    pub weight_keys: Vec<WeightKey>,
}

/// 物理权重根所有权记录
#[derive(Debug, Clone)]
pub struct WeightOwner {
    pub weight_id: String,
    pub key: WeightKey,
    pub generation: u64,
    pub ref_count: u32,
    pub state: WeightState,
    pub last_used_at: Instant,
}

/// 账本配置参数
#[derive(Debug, Clone)]
pub struct LedgerConfig {
    pub max_instances_per_core: u32,
    pub max_offline_workers: u32,
    pub max_quarantined_workers: usize,
    pub boot_id: String,
}

impl Default for LedgerConfig {
    fn default() -> Self {
        Self {
            max_instances_per_core: 4,
            max_offline_workers: 2,
            max_quarantined_workers: 5,
            boot_id: "boot-1".to_string(),
        }
    }
}

/// 内部保护的账本内存状态
#[derive(Debug)]
pub struct LedgerState {
    pub inventory: DeviceInventory,
    pub reservations: HashMap<ReservationId, ExecutionReservation>,
    pub weights: HashMap<WeightKey, WeightOwner>,
    pub core_allocations: HashMap<(String, u32), u32>,
    pub offline_active_count: u32,
    pub round_robin_counter: u64,
    pub config: LedgerConfig,
}

impl LedgerState {
    pub fn new(inventory: DeviceInventory, config: LedgerConfig) -> Self {
        Self {
            inventory,
            reservations: HashMap::new(),
            weights: HashMap::new(),
            core_allocations: HashMap::new(),
            offline_active_count: 0,
            round_robin_counter: 0,
            config,
        }
    }
}

/// 宿主放置与内存账本
pub struct PlacementLedger {
    state: Mutex<LedgerState>,
}

impl std::fmt::Debug for PlacementLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f.debug_struct("PlacementLedger")
            .field("active_reservations", &guard.reservations.len())
            .field("active_weights", &guard.weights.len())
            .field("topology_generation", &guard.inventory.topology_generation)
            .finish()
    }
}

impl PlacementLedger {
    /// 创建新的账本实例
    pub fn new(inventory: DeviceInventory, config: LedgerConfig) -> Self {
        Self {
            state: Mutex::new(LedgerState::new(inventory, config)),
        }
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
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // 1. 检查防雪崩熔断器：隔离线程是否超出阈值
        let current_quarantined = quarantined_workers_count();
        if current_quarantined >= guard.config.max_quarantined_workers {
            return Err(PlacementError::QuarantineBreakerTriggered);
        }

        // 2. 求解核心分配
        let solver_opts = super::solver::SolverOptions {
            is_offline,
            max_instances_per_core: guard.config.max_instances_per_core,
            max_offline_workers: guard.config.max_offline_workers,
            round_robin_counter: guard.round_robin_counter,
        };
        let decision = PlacementSolver::solve(
            &guard.inventory,
            intent,
            &guard.core_allocations,
            guard.offline_active_count,
            solver_opts,
        )?;

        guard.round_robin_counter = guard.round_robin_counter.wrapping_add(1);

        // 3. 生成全局唯一 reservation 标识
        let raw_id = RESERVATION_SEQ.fetch_add(1, Ordering::Relaxed);
        let res_id = ReservationId(raw_id);
        let res_str = format!("{}:res-{}", guard.config.boot_id, raw_id);

        // 4. 更新核心分配统计
        if decision.core_mask > 0 {
            let key = (decision.device_id.clone(), decision.core_mask);
            let count = guard.core_allocations.entry(key).or_insert(0);
            *count = count.saturating_add(1);
        }
        if is_offline {
            guard.offline_active_count = guard.offline_active_count.saturating_add(1);
        }

        // 5. 权重账本登记与引用计数计算
        let mut weight_keys = Vec::new();
        let mut weight_bindings = Vec::new();
        let mut needs_weight_init = false;

        for m_key in model_keys {
            let w_key = WeightKey {
                model_key: m_key.clone(),
                device_id: decision.device_id.clone(),
                package_generation,
            };

            weight_keys.push(w_key.clone());

            let owner = guard.weights.entry(w_key.clone()).or_insert_with(|| {
                needs_weight_init = true;
                WeightOwner {
                    weight_id: format!("w-{}-{}", m_key, raw_id),
                    key: w_key.clone(),
                    generation: 1,
                    ref_count: 0,
                    state: WeightState::Initializing,
                    last_used_at: Instant::now(),
                }
            });

            // 增加权重借用引用
            owner.ref_count = owner.ref_count.saturating_add(1);
            owner.state = WeightState::Ready;
            owner.last_used_at = Instant::now();

            weight_bindings.push(WireWeightBinding {
                model_key: m_key.clone(),
                weight_id: owner.weight_id.clone(),
                generation: owner.generation,
            });
        }

        let reservation = ExecutionReservation {
            id: res_id,
            reservation_string: res_str.clone(),
            instance_id: instance_id.to_string(),
            group_id: group_id.to_string(),
            generation: 1,
            topology_generation: guard.inventory.topology_generation,
            decision: decision.clone(),
            state: ExecutionState::Reserved,
            is_offline,
            weight_keys,
        };

        guard.reservations.insert(res_id, reservation.clone());

        let wire_meta = WirePlacementMetadata {
            version: 1,
            reservation_id: res_str,
            group_id: group_id.to_string(),
            generation: 1,
            device_id: decision.device_id,
            runtime_device_index: decision.runtime_device_index,
            strategy: decision.strategy,
            core_mask: decision.core_mask,
            required: true,
            weight_sharing: "required".to_string(),
            weight_bindings,
        };

        Ok((reservation, wire_meta, needs_weight_init))
    }

    /// 标记预留项就绪 (Ready)
    pub fn mark_ready(&self, id: ReservationId) -> Result<(), PlacementError> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let res = guard
            .reservations
            .get_mut(&id)
            .ok_or(PlacementError::ReservationNotFound(id.0))?;
        res.state = ExecutionState::Ready;
        Ok(())
    }

    /// 标记预留项隔离 (Quarantined)
    pub fn mark_quarantined(&self, id: ReservationId) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(res) = guard.reservations.get_mut(&id) {
            res.state = ExecutionState::Quarantined;
        }
    }

    /// 释放预留项并回收配额（旧代际资源无论拓扑是否更新，均可安全释放）
    pub fn release_placement(
        &self,
        id: ReservationId,
    ) -> Result<ExecutionReservation, PlacementError> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let res = guard
            .reservations
            .remove(&id)
            .ok_or(PlacementError::ReservationNotFound(id.0))?;

        // 1. 回收核心配额
        if res.decision.core_mask > 0 {
            let key = (res.decision.device_id.clone(), res.decision.core_mask);
            if let Some(count) = guard.core_allocations.get_mut(&key) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    guard.core_allocations.remove(&key);
                }
            }
        }

        // 2. 回收离线计数
        if res.is_offline {
            guard.offline_active_count = guard.offline_active_count.saturating_sub(1);
        }

        // 3. 递减关联物理权重的引用计数
        for w_key in &res.weight_keys {
            if let Some(owner) = guard.weights.get_mut(w_key) {
                owner.ref_count = owner.ref_count.saturating_sub(1);
                if owner.ref_count == 0 {
                    // 转入 Cooling 冷却期，等待退火
                    owner.state = WeightState::Cooling;
                    owner.last_used_at = Instant::now();
                }
            }
        }

        Ok(res)
    }

    /// 更新设备拓扑并推进代际
    pub fn update_inventory(&self, new_inventory: DeviceInventory) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.inventory = new_inventory;
    }

    /// 获取指定预留的状态
    pub fn get_reservation(&self, id: ReservationId) -> Option<ExecutionReservation> {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.reservations.get(&id).cloned()
    }

    /// 获取活跃实例总数
    pub fn active_reservations_count(&self) -> usize {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.reservations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ledger_reserve_and_release() {
        let inv = DeviceInventory::mock_rk3588();
        let ledger = PlacementLedger::new(inv, LedgerConfig::default());

        let intent = AffinityIntent::Auto {
            policy: "spread".to_string(),
        };
        let (res, _wire, needs_init) = ledger
            .reserve_placement(
                "cam-1",
                "grp-1",
                &intent,
                &["yolov8n".to_string()],
                1,
                false,
            )
            .expect("预留应成功");

        assert_eq!(res.decision.core_mask, 1);
        assert!(needs_init);
        assert_eq!(ledger.active_reservations_count(), 1);

        // 标记就绪
        ledger.mark_ready(res.id).expect("标记 Ready 应成功");
        let query = ledger.get_reservation(res.id).expect("应查到预留项");
        assert_eq!(query.state, ExecutionState::Ready);

        // 释放
        let released = ledger.release_placement(res.id).expect("释放应成功");
        assert_eq!(released.id, res.id);
        assert_eq!(ledger.active_reservations_count(), 0);
    }
}
