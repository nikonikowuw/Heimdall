//! NPU 放置求解算法 (PlacementSolver)
//!
//! 负责根据设备拓扑与当前分配状态，为新实例计算确定的核心分配决策。

use std::collections::HashMap;
use types::placement::AffinityIntent;

use super::inventory::DeviceInventory;

/// 放置决策求解错误分类
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PlacementError {
    #[error("未找到指定的 NPU 设备: {0}")]
    DeviceNotFound(String),

    #[error("当前系统无可用 NPU 设备")]
    NoDeviceAvailable,

    #[error("指定的 NPU 核心索引越界或不支持: core {core_index} on {device_id}")]
    InvalidCoreIndex { device_id: String, core_index: u32 },

    #[error("设备 {0} 物理上不支持指定核心绑定")]
    CorePinningNotSupported(String),

    #[error("核心 {core_index} 实例配额已满 (上限: {max})")]
    CoreQuotaExceeded { core_index: u32, max: u32 },

    #[error("设备 {device_id} 全部核心配额已满 (上限: {max})")]
    DeviceQuotaExceeded { device_id: String, max: u32 },

    #[error("离线推理 Worker 配额已满 (上限: {max})")]
    OfflineQuotaExceeded { max: u32 },

    #[error("系统存在过多隔离异常工作线程，触发防雪崩熔断")]
    QuarantineBreakerTriggered,

    #[error("未找到有效的资源预留: {0}")]
    ReservationNotFound(u64),
}

/// 放置求解结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementDecision {
    /// 目标设备 ID
    pub device_id: String,
    /// 运行时设备索引 (从 0 开始)
    pub runtime_device_index: u32,
    /// 目标核心位掩码 (如 1, 2, 4，或 0 表示由底层 runtime 自动调度)
    pub core_mask: u32,
    /// 放置策略 ("pinned" | "runtimeManaged" | "degraded")
    pub strategy: String,
}

/// 放置求解配置选项
#[derive(Debug, Clone, Copy)]
pub struct SolverOptions {
    pub is_offline: bool,
    pub max_instances_per_core: u32,
    pub max_offline_workers: u32,
    pub round_robin_counter: u64,
}

/// 纯计算的放置求解器
#[derive(Debug, Clone, Copy)]
pub struct PlacementSolver;

impl PlacementSolver {
    /// 计算最优核心分配
    pub fn solve(
        inventory: &DeviceInventory,
        intent: &AffinityIntent,
        core_allocations: &HashMap<(String, u32), u32>,
        offline_active_count: u32,
        options: SolverOptions,
    ) -> Result<PlacementDecision, PlacementError> {
        // 1. 检查离线配额隔离
        if options.is_offline && offline_active_count >= options.max_offline_workers {
            return Err(PlacementError::OfflineQuotaExceeded {
                max: options.max_offline_workers,
            });
        }

        // 2. 检查物理设备是否为空（无 NPU 时降级处理）
        if inventory.is_empty() {
            return Ok(PlacementDecision {
                device_id: "cpu".to_string(),
                runtime_device_index: 0,
                core_mask: 0,
                strategy: "degraded".to_string(),
            });
        }

        match intent {
            AffinityIntent::Manual {
                device_id,
                core_index,
            } => {
                let dev = inventory
                    .find_device(device_id)
                    .ok_or_else(|| PlacementError::DeviceNotFound(device_id.clone()))?;

                if !dev.supports_core_pinning {
                    return Err(PlacementError::CorePinningNotSupported(device_id.clone()));
                }

                if !dev.is_core_index_valid(*core_index) {
                    return Err(PlacementError::InvalidCoreIndex {
                        device_id: device_id.clone(),
                        core_index: *core_index,
                    });
                }

                let core_mask = 1 << *core_index;
                let active = core_allocations
                    .get(&(device_id.clone(), core_mask))
                    .copied()
                    .unwrap_or(0);

                if active >= options.max_instances_per_core {
                    return Err(PlacementError::CoreQuotaExceeded {
                        core_index: *core_index,
                        max: options.max_instances_per_core,
                    });
                }

                Ok(PlacementDecision {
                    device_id: device_id.clone(),
                    runtime_device_index: 0,
                    core_mask,
                    strategy: "pinned".to_string(),
                })
            }
            AffinityIntent::Auto { policy } => {
                let dev = inventory
                    .primary_device()
                    .ok_or(PlacementError::NoDeviceAvailable)?;

                // 若硬件不支持设核（如单核 RK3568），退化为 runtimeManaged
                if !dev.supports_core_pinning || dev.core_count <= 1 {
                    return Ok(PlacementDecision {
                        device_id: dev.device_id.clone(),
                        runtime_device_index: 0,
                        core_mask: 0,
                        strategy: "runtimeManaged".to_string(),
                    });
                }

                // 收集所有有效核心与其负载
                let mut valid_cores = Vec::new();
                for i in 0..dev.core_count {
                    if dev.is_core_index_valid(i) {
                        let mask = 1 << i;
                        let load = core_allocations
                            .get(&(dev.device_id.clone(), mask))
                            .copied()
                            .unwrap_or(0);
                        if load < options.max_instances_per_core {
                            valid_cores.push((i, mask, load));
                        }
                    }
                }

                if valid_cores.is_empty() {
                    return Err(PlacementError::DeviceQuotaExceeded {
                        device_id: dev.device_id.clone(),
                        max: options
                            .max_instances_per_core
                            .saturating_mul(dev.core_count),
                    });
                }

                let chosen_mask = match policy.as_str() {
                    "pack" => {
                        // 寻找最大负载（紧凑打包）
                        let max_load = valid_cores.iter().map(|(_, _, l)| *l).max().unwrap_or(0);
                        let candidates: Vec<&(u32, u32, u32)> = valid_cores
                            .iter()
                            .filter(|(_, _, l)| *l == max_load)
                            .collect();

                        candidates[0].1
                    }
                    _ => {
                        // 默认 Spread：寻找最小负载
                        let min_load = valid_cores.iter().map(|(_, _, l)| *l).min().unwrap_or(0);
                        let candidates: Vec<&(u32, u32, u32)> = valid_cores
                            .iter()
                            .filter(|(_, _, l)| *l == min_load)
                            .collect();

                        // 负载相同时，基于 round_robin_counter 按物理核心索引顺序平滑轮转
                        let target_idx =
                            (options.round_robin_counter % (dev.core_count as u64)) as u32;
                        let selected = candidates
                            .iter()
                            .find(|(idx, _, _)| *idx >= target_idx)
                            .copied()
                            .unwrap_or(candidates[0]);

                        selected.1
                    }
                };

                Ok(PlacementDecision {
                    device_id: dev.device_id.clone(),
                    runtime_device_index: 0,
                    core_mask: chosen_mask,
                    strategy: "pinned".to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solver_manual_success() {
        let inv = DeviceInventory::mock_rk3588();
        let intent = AffinityIntent::Manual {
            device_id: "rknn-npu0".to_string(),
            core_index: 1,
        };
        let allocations = HashMap::new();
        let opts = SolverOptions {
            is_offline: false,
            max_instances_per_core: 4,
            max_offline_workers: 2,
            round_robin_counter: 0,
        };
        let res = PlacementSolver::solve(&inv, &intent, &allocations, 0, opts)
            .expect("manual 分配应成功");
        assert_eq!(res.core_mask, 2);
        assert_eq!(res.strategy, "pinned");
    }

    #[test]
    fn test_solver_manual_invalid_core() {
        let inv = DeviceInventory::mock_rk3588();
        let intent = AffinityIntent::Manual {
            device_id: "rknn-npu0".to_string(),
            core_index: 5,
        };
        let allocations = HashMap::new();
        let opts = SolverOptions {
            is_offline: false,
            max_instances_per_core: 4,
            max_offline_workers: 2,
            round_robin_counter: 0,
        };
        let err = PlacementSolver::solve(&inv, &intent, &allocations, 0, opts)
            .expect_err("越界核心索引应报错");
        assert!(matches!(err, PlacementError::InvalidCoreIndex { .. }));
    }

    #[test]
    fn test_solver_manual_unsupported_on_rk3568() {
        let inv = DeviceInventory::mock_rk3568();
        let intent = AffinityIntent::Manual {
            device_id: "rknn-npu0".to_string(),
            core_index: 0,
        };
        let allocations = HashMap::new();
        let opts = SolverOptions {
            is_offline: false,
            max_instances_per_core: 4,
            max_offline_workers: 2,
            round_robin_counter: 0,
        };
        let err = PlacementSolver::solve(&inv, &intent, &allocations, 0, opts)
            .expect_err("单核设备设核应报错");
        assert!(matches!(err, PlacementError::CorePinningNotSupported(_)));
    }

    #[test]
    fn test_solver_auto_spread_round_robin() {
        let inv = DeviceInventory::mock_rk3588();
        let intent = AffinityIntent::Auto {
            policy: "spread".to_string(),
        };
        let mut allocations = HashMap::new();

        let opts1 = SolverOptions {
            is_offline: false,
            max_instances_per_core: 4,
            max_offline_workers: 2,
            round_robin_counter: 0,
        };
        // 第一次分配：3 核均空闲，counter=0 -> core 0 (mask=1)
        let r1 =
            PlacementSolver::solve(&inv, &intent, &allocations, 0, opts1).expect("r1 分配应成功");
        assert_eq!(r1.core_mask, 1);
        allocations.insert(("rknn-npu0".to_string(), 1), 1);

        let opts2 = SolverOptions {
            is_offline: false,
            max_instances_per_core: 4,
            max_offline_workers: 2,
            round_robin_counter: 1,
        };
        // 第二次分配：core 0 负载=1，core 1/2 负载=0，counter=1 -> core 1 (mask=2)
        let r2 =
            PlacementSolver::solve(&inv, &intent, &allocations, 0, opts2).expect("r2 分配应成功");
        assert_eq!(r2.core_mask, 2);
        allocations.insert(("rknn-npu0".to_string(), 2), 1);

        let opts3 = SolverOptions {
            is_offline: false,
            max_instances_per_core: 4,
            max_offline_workers: 2,
            round_robin_counter: 2,
        };
        // 第三次分配：core 0,1 负载=1，core 2 负载=0 -> core 2 (mask=4)
        let r3 =
            PlacementSolver::solve(&inv, &intent, &allocations, 0, opts3).expect("r3 分配应成功");
        assert_eq!(r3.core_mask, 4);
    }
}
