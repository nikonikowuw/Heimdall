//! 人员底库重型任务互斥闸门
//!
//! 「批量导入」与「全量特征重提取」都要反复调用 NPU 做人脸检测与特征提取。
//! 在 RK3568 这类单核 NPU 边缘设备上并行执行会直接挤压常驻视频流的推理时间片，
//! 因此二者必须全局互斥。
//!
//! 用单一许可的结构化闸门而非在调用点分散写 `if a.is_running() || b.is_running()`：
//! 互斥不变量由类型承载，新增重型任务时只需申请同一个闸门，无法因遗漏检查而失效。

use std::sync::{Arc, Mutex};

use crate::error::ApiError;

/// 占用闸门的重型任务类别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceTaskKind {
    /// 人员批量导入
    Import,
    /// 全量底库特征重新提取
    Reextract,
}

impl MaintenanceTaskKind {
    /// 面向管理员的任务名称
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Import => "人员批量导入",
            Self::Reextract => "人脸特征重新提取",
        }
    }
}

/// 全局单许可互斥闸门
#[derive(Debug, Clone, Default)]
pub struct MaintenanceGate {
    holder: Arc<Mutex<Option<MaintenanceTaskKind>>>,
}

impl MaintenanceGate {
    pub fn new() -> Self {
        Self {
            holder: Arc::new(Mutex::new(None)),
        }
    }

    /// 尝试独占闸门；已有任务持有时返回 409 冲突
    ///
    /// 返回的守卫随任务结束析构并释放闸门；即便任务 panic，`Drop` 也会执行，
    /// 不会把闸门永久锁死。
    pub fn acquire(&self, kind: MaintenanceTaskKind) -> Result<MaintenanceGuard, ApiError> {
        let mut holder = self
            .holder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(current) = *holder {
            return Err(ApiError::FaceExtractionConflict(format!(
                "当前有{}任务正在后台执行中，请稍候再试",
                current.label()
            )));
        }

        *holder = Some(kind);
        Ok(MaintenanceGuard {
            gate: self.clone(),
            kind,
        })
    }

    /// 当前持闸任务类别（无任务时为空）
    pub fn current(&self) -> Option<MaintenanceTaskKind> {
        *self
            .holder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 闸门独占守卫（RAII 释放）
#[derive(Debug)]
pub struct MaintenanceGuard {
    gate: MaintenanceGate,
    kind: MaintenanceTaskKind,
}

impl MaintenanceGuard {
    /// 占用的任务类别
    pub fn kind(&self) -> MaintenanceTaskKind {
        self.kind
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        let mut holder = self
            .gate
            .holder
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // 仅当持有人仍是自己时才释放，避免误清后来者的占用
        if *holder == Some(self.kind) {
            *holder = None;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_rejected_while_first_is_held() {
        let gate = MaintenanceGate::new();
        let first = gate.acquire(MaintenanceTaskKind::Import).unwrap();

        let err = gate.acquire(MaintenanceTaskKind::Reextract).unwrap_err();
        assert!(matches!(err, ApiError::FaceExtractionConflict(_)));
        assert!(err.to_string().contains("人员批量导入"));

        drop(first);
        assert!(gate.acquire(MaintenanceTaskKind::Reextract).is_ok());
    }

    #[test]
    fn guard_release_is_idempotent_across_task_kinds() {
        let gate = MaintenanceGate::new();
        {
            let _guard = gate.acquire(MaintenanceTaskKind::Reextract).unwrap();
            assert_eq!(gate.current(), Some(MaintenanceTaskKind::Reextract));
        }
        assert_eq!(gate.current(), None);

        let _second = gate.acquire(MaintenanceTaskKind::Import).unwrap();
        assert_eq!(gate.current(), Some(MaintenanceTaskKind::Import));
    }
}
