use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 操作审计日志模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLog {
    pub id: u64,
    pub username: String,
    pub module: String,
    pub action: String,
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    pub status_code: u16,
    pub duration_ms: i64,
    pub ip: String,
    pub user_agent: String,
    pub created_at: DateTime<Utc>,
}

// ===========================================================================
// 运维事件日志 (Operational Log)
//
// 与操作审计日志 (OperationLog) 区分：
// - OperationLog：用户操作记录（who did what），由 HTTP 中间件写入
// - OpEvent：系统状态变迁（what happened），由业务模块主动调用录制
// ===========================================================================

/// 运维事件枚举——每种事件携带的字段在类型层面固定
#[derive(Debug, Clone)]
pub enum OpEvent {
    // ── 生命周期 ──
    ServiceStarted {
        version: String,
        port: u16,
    },
    ServiceStopped,

    // ── 摄像头状态变迁 ──
    CameraOnline {
        camera_id: String,
    },
    CameraOffline {
        camera_id: String,
        /// 仅填写真实视频帧 PTS；探活结果不含帧时间时保持未知。
        last_frame_ts: Option<i64>,
        reason: String,
    },

    // ── 任务状态 ──
    TaskStarted {
        task_id: i64,
    },
    TaskStopped {
        task_id: i64,
    },
    TaskCompleted {
        task_id: i64,
    },
    TaskFailed {
        task_id: i64,
        error: String,
    },
    TaskDegraded {
        task_id: i64,
        reason: String,
    },

    // ── 告警 ──
    AlarmTriggered {
        alarm_id: String,
        camera_id: String,
        rule_type: String,
    },

    // ── 硬件与推理 ──
    AlgoLoaded {
        algo_id: String,
        platform: String,
    },
    AlgoUnloaded {
        algo_id: String,
    },
    AlgoLoadFailed {
        algo_id: String,
        error: String,
    },
    AlgoSandboxFailed {
        algo_id: String,
        error: String,
    },
    NpuInitFailed {
        backend: String,
        error: String,
    },
    VpuExhausted {
        camera_id: String,
    },

    // ── 存储 ──
    StorageWatermark {
        level: String,
        free_ratio: f64,
    },
    StorageEviction {
        evicted: u64,
        freed_mb: u64,
    },
}

impl OpEvent {
    /// 事件标记 (写入数据库的 event 字段)
    pub fn tag(&self) -> &'static str {
        match self {
            Self::ServiceStarted { .. } => "service_started",
            Self::ServiceStopped => "service_stopped",
            Self::CameraOnline { .. } => "camera_online",
            Self::CameraOffline { .. } => "camera_offline",
            Self::TaskStarted { .. } => "task_started",
            Self::TaskStopped { .. } => "task_stopped",
            Self::TaskCompleted { .. } => "task_completed",
            Self::TaskFailed { .. } => "task_failed",
            Self::TaskDegraded { .. } => "task_degraded",
            Self::AlarmTriggered { .. } => "alarm_triggered",
            Self::AlgoLoaded { .. } => "algo_loaded",
            Self::AlgoUnloaded { .. } => "algo_unloaded",
            Self::AlgoLoadFailed { .. } => "algo_load_failed",
            Self::AlgoSandboxFailed { .. } => "algo_sandbox_failed",
            Self::NpuInitFailed { .. } => "npu_init_failed",
            Self::VpuExhausted { .. } => "vpu_exhausted",
            Self::StorageWatermark { .. } => "storage_watermark",
            Self::StorageEviction { .. } => "storage_eviction",
        }
    }

    /// 日志级别 (运维日志只有 error / warn / info)
    pub fn level(&self) -> &'static str {
        match self {
            Self::NpuInitFailed { .. }
            | Self::AlgoLoadFailed { .. }
            | Self::AlgoSandboxFailed { .. }
            | Self::VpuExhausted { .. } => "error",
            Self::StorageWatermark { level, .. } if level == "normal" => "info",
            Self::CameraOffline { .. }
            | Self::TaskFailed { .. }
            | Self::TaskDegraded { .. }
            | Self::StorageWatermark { .. } => "warn",
            _ => "info",
        }
    }

    /// 可选关联的摄像头 ID
    pub fn camera_id(&self) -> Option<&str> {
        match self {
            Self::CameraOnline { camera_id, .. }
            | Self::CameraOffline { camera_id, .. }
            | Self::AlarmTriggered { camera_id, .. }
            | Self::VpuExhausted { camera_id } => Some(camera_id.as_str()),
            _ => None,
        }
    }

    pub fn error_detail(&self) -> Option<&str> {
        match self {
            Self::CameraOffline { reason, .. }
            | Self::TaskFailed { error: reason, .. }
            | Self::TaskDegraded { reason, .. }
            | Self::AlgoLoadFailed { error: reason, .. }
            | Self::AlgoSandboxFailed { error: reason, .. }
            | Self::NpuInitFailed { error: reason, .. } => Some(reason),
            _ => None,
        }
    }

    pub fn task_id(&self) -> Option<i64> {
        match self {
            Self::TaskStarted { task_id }
            | Self::TaskStopped { task_id }
            | Self::TaskCompleted { task_id }
            | Self::TaskFailed { task_id, .. }
            | Self::TaskDegraded { task_id, .. } => Some(*task_id),
            _ => None,
        }
    }

    pub fn algo_id(&self) -> Option<&str> {
        match self {
            Self::AlgoLoaded { algo_id, .. }
            | Self::AlgoUnloaded { algo_id }
            | Self::AlgoLoadFailed { algo_id, .. }
            | Self::AlgoSandboxFailed { algo_id, .. } => Some(algo_id),
            _ => None,
        }
    }

    /// 事件归属模块 (写入数据库的 target 字段)
    pub fn target(&self) -> &'static str {
        match self {
            Self::ServiceStarted { .. } | Self::ServiceStopped => "system",
            Self::CameraOnline { .. } | Self::CameraOffline { .. } => "media",
            Self::TaskStarted { .. }
            | Self::TaskStopped { .. }
            | Self::TaskCompleted { .. }
            | Self::TaskFailed { .. }
            | Self::TaskDegraded { .. } => "pipeline",
            Self::AlarmTriggered { .. } => "rule",
            Self::AlgoLoaded { .. }
            | Self::AlgoUnloaded { .. }
            | Self::AlgoLoadFailed { .. }
            | Self::AlgoSandboxFailed { .. } => "infer",
            Self::NpuInitFailed { .. } | Self::VpuExhausted { .. } => "hardware",
            Self::StorageWatermark { .. } | Self::StorageEviction { .. } => "storage",
        }
    }

    /// 人类可读消息 (中文短消息)
    pub fn message(&self) -> String {
        match self {
            Self::ServiceStarted { version, port } => {
                format!("Heimdall {version} 服务启动，监听端口 {port}")
            }
            Self::ServiceStopped => "Heimdall 服务已停止".to_string(),
            Self::CameraOnline { .. } => "摄像头连接就绪".to_string(),
            Self::CameraOffline { .. } => "摄像头离线".to_string(),
            Self::TaskStarted { task_id } => {
                format!("任务 #{task_id} 已启动")
            }
            Self::TaskStopped { .. } => "任务已停止".to_string(),
            Self::TaskCompleted { task_id } => {
                format!("任务 #{task_id} 已完成")
            }
            Self::TaskFailed { task_id, error } => {
                format!("任务 #{task_id} 失败: {error}")
            }
            Self::TaskDegraded { task_id, reason } => {
                format!("任务 #{task_id} 降级运行: {reason}")
            }
            Self::AlarmTriggered {
                camera_id,
                rule_type,
                ..
            } => {
                format!("摄像头 {camera_id} 触发告警: {rule_type}")
            }
            Self::AlgoLoaded {
                algo_id, platform, ..
            } => {
                format!("算法包 {algo_id} 已加载 ({platform})")
            }
            Self::AlgoUnloaded { algo_id } => {
                format!("算法包 {algo_id} 已卸载")
            }
            Self::AlgoLoadFailed { .. } => "算法包加载失败".to_string(),
            Self::AlgoSandboxFailed { algo_id, error } => {
                format!("算法包 {algo_id} 沙箱自检失败: {error}")
            }
            Self::NpuInitFailed { backend, error } => {
                format!("NPU 后端 {backend} 初始化失败: {error}")
            }
            Self::VpuExhausted { .. } => "VPU 抓拍通道配额耗尽".to_string(),
            Self::StorageWatermark { level, free_ratio } => {
                format!("存储水位 {level}: 剩余 {:.1}%", free_ratio * 100.0)
            }
            Self::StorageEviction { evicted, freed_mb } => {
                format!("淘汰 {evicted} 条记录，释放 {freed_mb}MB")
            }
        }
    }

    /// 变体特有字段序列化为 JSON (写入 extra_json 列)
    pub fn extra_json(&self) -> Option<String> {
        let value = match self {
            Self::ServiceStarted { version, port } => {
                serde_json::json!({ "version": version, "port": port })
            }
            Self::CameraOffline {
                last_frame_ts,
                reason,
                ..
            } => {
                serde_json::json!({ "last_frame_ts": last_frame_ts, "reason": reason })
            }
            Self::TaskStarted { task_id }
            | Self::TaskStopped { task_id }
            | Self::TaskCompleted { task_id } => serde_json::json!({ "task_id": task_id }),
            Self::TaskFailed { task_id, error } => {
                serde_json::json!({ "task_id": task_id, "error": error })
            }
            Self::TaskDegraded { task_id, reason } => {
                serde_json::json!({ "task_id": task_id, "reason": reason })
            }
            Self::AlarmTriggered { alarm_id, .. } => {
                serde_json::json!({ "alarm_id": alarm_id })
            }
            Self::AlgoLoaded { algo_id, platform } => {
                serde_json::json!({ "algo_id": algo_id, "platform": platform })
            }
            Self::AlgoUnloaded { algo_id } => {
                serde_json::json!({ "algo_id": algo_id })
            }
            Self::AlgoLoadFailed { algo_id, error }
            | Self::AlgoSandboxFailed { algo_id, error } => {
                serde_json::json!({ "algo_id": algo_id, "error": error })
            }
            Self::NpuInitFailed { backend, error } => {
                serde_json::json!({ "backend": backend, "error": error })
            }
            Self::VpuExhausted { camera_id } => {
                serde_json::json!({ "camera_id": camera_id })
            }
            Self::StorageWatermark { free_ratio, .. } => {
                serde_json::json!({ "free_ratio": free_ratio })
            }
            Self::StorageEviction { evicted, freed_mb } => {
                serde_json::json!({ "evicted": evicted, "freed_mb": freed_mb })
            }
            // 无结构化附加字段的变体
            Self::ServiceStopped | Self::CameraOnline { .. } => {
                return None;
            }
        };
        serde_json::to_string(&value).ok()
    }

    /// 将任务状态变化映射到适合运维日志展示的生命周期事件。
    pub fn task_status_transition(
        task_id: i64,
        previous: i32,
        current: crate::TaskStatus,
        reason: &str,
    ) -> Option<Self> {
        if crate::TaskStatus::from_i32(previous) == Some(current) {
            return None;
        }

        match current {
            crate::TaskStatus::Running => Some(Self::TaskStarted { task_id }),
            crate::TaskStatus::Stopped => Some(Self::TaskStopped { task_id }),
            crate::TaskStatus::Error => Some(Self::TaskFailed {
                task_id,
                error: reason.to_string(),
            }),
            crate::TaskStatus::Degraded | crate::TaskStatus::Reconnecting => {
                Some(Self::TaskDegraded {
                    task_id,
                    reason: reason.to_string(),
                })
            }
            crate::TaskStatus::Starting => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OpEvent;
    use crate::TaskStatus;

    #[test]
    fn task_status_transition_only_emits_changed_operational_states() {
        assert!(matches!(
            OpEvent::task_status_transition(
                7,
                TaskStatus::Stopped.as_i32(),
                TaskStatus::Running,
                "",
            ),
            Some(OpEvent::TaskStarted { task_id: 7 })
        ));
        assert!(matches!(
            OpEvent::task_status_transition(
                7,
                TaskStatus::Running.as_i32(),
                TaskStatus::Stopped,
                "",
            ),
            Some(OpEvent::TaskStopped { task_id: 7 })
        ));
        assert!(OpEvent::task_status_transition(
            7,
            TaskStatus::Running.as_i32(),
            TaskStatus::Running,
            "",
        )
        .is_none());
    }

    #[test]
    fn vpu_exhaustion_is_correlated_to_its_camera() {
        let event = OpEvent::VpuExhausted {
            camera_id: "CAM-01".to_string(),
        };
        assert_eq!(event.tag(), "vpu_exhausted");
        assert_eq!(event.camera_id(), Some("CAM-01"));
        assert_eq!(event.target(), "hardware");
    }

    #[test]
    fn algo_load_failure_keeps_details_out_of_the_message() {
        let event = OpEvent::AlgoLoadFailed {
            algo_id: "detector".to_string(),
            error: "missing entry point".to_string(),
        };
        assert_eq!(event.message(), "算法包加载失败");
        assert_eq!(event.error_detail(), Some("missing entry point"));
        let details: serde_json::Value =
            serde_json::from_str(&event.extra_json().expect("algorithm details"))
                .expect("valid event details");
        assert_eq!(details["algo_id"], "detector");
        assert_eq!(details["error"], "missing entry point");
    }

    #[test]
    fn camera_offline_details_preserve_unknown_frame_time() {
        let event = OpEvent::CameraOffline {
            camera_id: "CAM-01".to_string(),
            last_frame_ts: None,
            reason: "probe timeout".to_string(),
        };
        assert_eq!(event.message(), "摄像头离线");
        let details: serde_json::Value =
            serde_json::from_str(&event.extra_json().expect("camera details"))
                .expect("valid event details");
        assert!(details["last_frame_ts"].is_null());
        assert_eq!(details["reason"], "probe timeout");
    }
}
