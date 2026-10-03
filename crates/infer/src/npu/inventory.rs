//! NPU 设备拓扑探测与管理 (DeviceInventory)
//!
//! 提供系统 NPU 拓扑建模、可用核心掩码计算、以及拓扑代际追踪。

use super::{NpuDevice, NpuDeviceType};

/// 单个 NPU 设备的拓扑与能力描述
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTopology {
    /// 设备唯一标识符（如 "rknn-npu0", "ascend-0"）
    pub device_id: String,
    /// 硬件设备类型
    pub device_type: NpuDeviceType,
    /// 物理核心数
    pub core_count: u32,
    /// 支持的核心位掩码（例如 3 核为 0b111 = 7）
    pub supported_core_mask: u32,
    /// 硬件/驱动是否支持运行时指定核心绑定（rknn_set_core_mask）
    pub supports_core_pinning: bool,
    /// 设备总内存 (MB)
    pub memory_total_mb: u64,
}

impl DeviceTopology {
    /// 检查指定核心索引（0-based）是否在物理支持范围内
    pub fn is_core_index_valid(&self, core_index: u32) -> bool {
        if core_index >= 32 {
            return false;
        }
        (self.supported_core_mask & (1 << core_index)) != 0
    }

    /// 将核心索引转为单核心掩码
    pub fn core_mask_for_index(&self, core_index: u32) -> Option<u32> {
        if self.is_core_index_valid(core_index) {
            Some(1 << core_index)
        } else {
            None
        }
    }
}

/// 系统 NPU 拓扑清单，包含当前代际号
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInventory {
    /// 拓扑代际号（设备插拔、驱动重载或降级时自增）
    pub topology_generation: u64,
    /// 探测到的可用 NPU 设备列表
    pub devices: Vec<DeviceTopology>,
}

impl DeviceInventory {
    /// 使用指定设备列表创建新清单，初始代际为 1
    pub fn new(devices: Vec<DeviceTopology>) -> Self {
        Self {
            topology_generation: 1,
            devices,
        }
    }

    /// 从已探测的底层 NpuDevice trait 抽象集合构建
    pub fn from_detected(detected: &[Box<dyn NpuDevice>]) -> Self {
        let devices = detected
            .iter()
            .map(|d| {
                let core_count = d.core_count();
                let (total_mb, _, _) = d.memory_info().unwrap_or((0, 0, 0));
                let dev_type = d.device_type();
                let supports_pinning = match dev_type {
                    NpuDeviceType::RockchipRknn => core_count > 1,
                    NpuDeviceType::AscendAcl => false,
                    NpuDeviceType::Unknown => false,
                };
                let mask = if core_count == 0 {
                    0
                } else if core_count >= 32 {
                    u32::MAX
                } else {
                    (1 << core_count) - 1
                };

                DeviceTopology {
                    device_id: d.device_id().to_string(),
                    device_type: dev_type,
                    core_count,
                    supported_core_mask: mask,
                    supports_core_pinning: supports_pinning,
                    memory_total_mb: total_mb,
                }
            })
            .collect();

        Self::new(devices)
    }

    /// 查找指定 device_id 的设备拓扑
    pub fn find_device(&self, device_id: &str) -> Option<&DeviceTopology> {
        self.devices.iter().find(|d| d.device_id == device_id)
    }

    /// 获取主 NPU 设备（第一个可用设备）
    pub fn primary_device(&self) -> Option<&DeviceTopology> {
        self.devices.first()
    }

    /// 是否无可用 NPU 设备
    pub fn is_empty(&self) -> bool {
        self.devices.is_empty()
    }

    /// 更新设备拓扑并自增拓扑代际
    pub fn advance_topology(&mut self, new_devices: Vec<DeviceTopology>) {
        self.devices = new_devices;
        self.topology_generation = self.topology_generation.saturating_add(1);
    }

    /// 构造 Mock RK3588 拓扑（3 核心，支持设核）
    pub fn mock_rk3588() -> Self {
        Self::new(vec![DeviceTopology {
            device_id: "rknn-npu0".to_string(),
            device_type: NpuDeviceType::RockchipRknn,
            core_count: 3,
            supported_core_mask: 0b111,
            supports_core_pinning: true,
            memory_total_mb: 8192,
        }])
    }

    /// 构造 Mock RK3576 拓扑（2 核心，支持设核）
    pub fn mock_rk3576() -> Self {
        Self::new(vec![DeviceTopology {
            device_id: "rknn-npu0".to_string(),
            device_type: NpuDeviceType::RockchipRknn,
            core_count: 2,
            supported_core_mask: 0b011,
            supports_core_pinning: true,
            memory_total_mb: 4096,
        }])
    }

    /// 构造 Mock RK3568 拓扑（1 核心，不支持设核）
    pub fn mock_rk3568() -> Self {
        Self::new(vec![DeviceTopology {
            device_id: "rknn-npu0".to_string(),
            device_type: NpuDeviceType::RockchipRknn,
            core_count: 1,
            supported_core_mask: 0b001,
            supports_core_pinning: false,
            memory_total_mb: 2048,
        }])
    }

    /// 构造 Mock 无硬件拓扑
    pub fn mock_empty() -> Self {
        Self::new(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_topology_core_validity() {
        let rk3588 = DeviceInventory::mock_rk3588();
        let dev = rk3588.primary_device().expect("应当有设备");
        assert!(dev.is_core_index_valid(0));
        assert!(dev.is_core_index_valid(1));
        assert!(dev.is_core_index_valid(2));
        assert!(!dev.is_core_index_valid(3));
        assert_eq!(dev.core_mask_for_index(0), Some(1));
        assert_eq!(dev.core_mask_for_index(1), Some(2));
        assert_eq!(dev.core_mask_for_index(2), Some(4));
        assert_eq!(dev.core_mask_for_index(3), None);
    }

    #[test]
    fn test_topology_generation_advance() {
        let mut inv = DeviceInventory::mock_rk3588();
        assert_eq!(inv.topology_generation, 1);
        inv.advance_topology(vec![]);
        assert_eq!(inv.topology_generation, 2);
        assert!(inv.is_empty());
    }
}
