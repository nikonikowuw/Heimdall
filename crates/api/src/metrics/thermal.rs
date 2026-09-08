//! 温度指标采集器 — Linux `/sys/class/thermal/thermal_zone*`

use types::ThermalMetrics;
#[cfg(target_os = "linux")]
use types::ThermalZone;

#[derive(Debug)]
pub struct ThermalCollector;

impl ThermalCollector {
    pub async fn collect() -> ThermalMetrics {
        #[cfg(target_os = "linux")]
        {
            Self::collect_linux().await
        }
        #[cfg(not(target_os = "linux"))]
        {
            ThermalMetrics::default()
        }
    }

    #[cfg(target_os = "linux")]
    async fn collect_linux() -> ThermalMetrics {
        let mut zones = Vec::new();
        for zone_id in 0..20 {
            if let Some(zone) = Self::read_thermal_zone(zone_id).await {
                zones.push(zone);
            }
        }
        let throttle_active = Self::detect_throttle_active(&zones).await;
        ThermalMetrics {
            zones,
            throttle_active,
        }
    }

    #[cfg(target_os = "linux")]
    async fn detect_throttle_active(zones: &[ThermalZone]) -> bool {
        // 1. 探测 cooling devices 是否处于非零节流状态
        for cdev_id in 0..10 {
            let cur_state_path = format!("/sys/class/thermal/cooling_device{}/cur_state", cdev_id);
            if let Ok(content) = tokio::fs::read_to_string(&cur_state_path).await {
                if let Ok(state) = content.trim().parse::<u32>() {
                    if state > 0 {
                        let type_path =
                            format!("/sys/class/thermal/cooling_device{}/type", cdev_id);
                        if let Ok(cdev_type) = tokio::fs::read_to_string(&type_path).await {
                            let lower = cdev_type.to_lowercase();
                            if lower.contains("cpufreq")
                                || lower.contains("processor")
                                || lower.contains("npu")
                            {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        // 2. 备选阈值判定：若核心 CPU/NPU 传感器温度持续超过 85°C
        zones
            .iter()
            .any(|z| (z.type_label == "cpu" || z.type_label == "npu") && z.temperature >= 85.0)
    }

    #[cfg(target_os = "linux")]
    async fn read_thermal_zone(zone_id: u32) -> Option<ThermalZone> {
        let base = format!("/sys/class/thermal/thermal_zone{}", zone_id);
        let type_str = tokio::fs::read_to_string(format!("{}/type", base))
            .await
            .ok()?
            .trim()
            .to_string();
        let temp_millideg = tokio::fs::read_to_string(format!("{}/temp", base))
            .await
            .ok()?
            .trim()
            .parse::<i32>()
            .ok()?;
        let temperature = temp_millideg as f32 / 1000.0;
        let type_label = Self::classify_thermal_type(&type_str);
        Some(ThermalZone {
            name: type_str,
            temperature,
            type_label,
        })
    }

    #[cfg(target_os = "linux")]
    fn classify_thermal_type(type_str: &str) -> String {
        let lower = type_str.to_lowercase();
        if lower.contains("cpu") || lower.contains("core") {
            "cpu".into()
        } else if lower.contains("npu") || lower.contains("nna") {
            "npu".into()
        } else if lower.contains("ddr") || lower.contains("memory") {
            "ddr".into()
        } else if lower.contains("gpu") {
            "gpu".into()
        } else if lower.contains("board") || lower.contains("skin") {
            "board".into()
        } else {
            "other".into()
        }
    }
}
