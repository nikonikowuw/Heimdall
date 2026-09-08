//! NPU 监控器
//!
//! 管理多个 NPU 设备，提供统一的监控接口。

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::broadcast;

use super::{detect_npu_devices, NpuDevice, NpuDeviceMetrics, NpuError};

/// NPU 监控事件
#[derive(Debug, Clone)]
pub struct NpuMonitorEvent {
    /// 所有设备的指标
    pub metrics: Vec<NpuDeviceMetrics>,
    /// 采集时间戳（Unix 毫秒）
    pub timestamp: i64,
}

/// NPU 监控器
pub struct NpuMonitor {
    /// 已探测到的设备
    devices: Vec<Box<dyn NpuDevice>>,
    /// 采样间隔
    poll_interval: Duration,
}

impl std::fmt::Debug for NpuMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NpuMonitor")
            .field("device_count", &self.devices.len())
            .field("poll_interval", &self.poll_interval)
            .finish()
    }
}

impl NpuMonitor {
    /// 创建新的 NPU 监控器
    ///
    /// 自动探测系统中的 NPU 设备
    pub fn new() -> Self {
        let devices = detect_npu_devices();
        let poll_interval = Duration::from_secs(2); // 默认 2 秒采样

        Self {
            devices,
            poll_interval,
        }
    }

    /// 创建指定采样间隔的 NPU 监控器
    pub fn with_interval(poll_interval: Duration) -> Self {
        let devices = detect_npu_devices();

        Self {
            devices,
            poll_interval,
        }
    }

    /// 获取设备数量
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }

    /// 获取所有设备的指标
    pub fn collect_all(&self) -> Vec<NpuDeviceMetrics> {
        self.devices
            .iter()
            .filter_map(|d| d.collect_metrics().ok())
            .collect()
    }

    /// 获取指定设备的指标
    pub fn collect_device(&self, index: usize) -> Result<NpuDeviceMetrics, NpuError> {
        self.devices
            .get(index)
            .ok_or(NpuError::DeviceNotFound)?
            .collect_metrics()
    }

    /// 启动后台监控任务
    ///
    /// 每隔 `poll_interval` 采集一次指标，通过 `broadcast::Sender` 广播
    pub fn start_polling(self: Arc<Self>, broadcaster: broadcast::Sender<NpuMonitorEvent>) {
        let interval = self.poll_interval;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;

                let metrics = self.collect_all();
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);

                let event = NpuMonitorEvent { metrics, timestamp };

                // 广播事件（忽略没有订阅者的错误）
                let _ = broadcaster.send(event);
            }
        });
    }

    /// 获取采样间隔
    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// 设置采样间隔（需要重启 polling 才能生效）
    pub fn set_poll_interval(&mut self, interval: Duration) {
        self.poll_interval = interval;
    }
}

impl Default for NpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// 全局 NPU 监控器单例（使用 std::sync::OnceLock）
use std::sync::OnceLock;

static GLOBAL_MONITOR: OnceLock<Arc<NpuMonitor>> = OnceLock::new();

/// 获取全局 NPU 监控器
///
/// 首次调用时自动探测设备并创建监控器
pub fn global_monitor() -> Arc<NpuMonitor> {
    GLOBAL_MONITOR
        .get_or_init(|| Arc::new(NpuMonitor::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_npu_monitor_creation() {
        let monitor = NpuMonitor::new();
        // 在没有 NPU 的测试环境中，设备数量应为 0
        assert_eq!(monitor.device_count(), 0);
    }

    #[test]
    fn test_npu_monitor_custom_interval() {
        let monitor = NpuMonitor::with_interval(Duration::from_secs(5));
        assert_eq!(monitor.poll_interval(), Duration::from_secs(5));
    }

    #[tokio::test]
    async fn test_npu_monitor_collect() {
        let monitor = NpuMonitor::new();
        let metrics = monitor.collect_all();
        // 在没有 NPU 的测试环境中，指标应为空
        assert!(metrics.is_empty());
    }
}
