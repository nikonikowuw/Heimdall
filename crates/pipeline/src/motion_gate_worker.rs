//! 运动门控专用 OS Worker：门控平台 FFI 的唯一执行位置
//!
//! DMA-BUF 载体上的门控评估会调用 RGA 硬件降采样（`improcess(IM_SYNC)`），并可能等待最多 100ms
//! 的源 DMA-BUF 可读栅障——这是平台 SDK / FFI 调用，按
//! [并发规范](../../../docs/nuwa/backend/concurrency-guidelines.md#执行归属)
//! 不得直接运行在 Tokio Worker 中。
//!
//! 本模块把 [`MotionGate`] 连同常驻的缩略图 DMA-BUF / RGA 句柄绑定到每路一个的固定 OS 线程：
//! 异步侧（解码循环）只做「提交帧 + 等待决策」，不持有任何硬件上下文，也不阻塞 Tokio 线程。
//!
//! 调用方串行提交（等上一帧决策返回后再提交下一帧），因此请求通道容量 1 仅作异常兜底；
//! 底层驱动挂死时通过客户端断路时限退出，绝不把解码循环拖住。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use types::{DetectionRule, FrameRef};

use crate::motion_gate::{MotionGate, MotionGateDecision};

/// 单帧门控评估的客户端断路时限
///
/// 正常路径是一次 RGA 同步降采样（预算约 0.5~1ms，源可读栅障等待上限 100ms），
/// 超出该时限只可能是底层驱动挂死：此时必须断路放行，而不是让解码循环被逐帧空等。
const GATE_EVAL_TIMEOUT: Duration = Duration::from_millis(500);

/// 工作线程停机有界等待上限（与解码线程停机口径一致，禁止无期限 `join`）
const GATE_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

/// 请求通道容量（调用方串行提交，容量 1 仅覆盖「上一帧仍在处理中」的异常时序）
const GATE_REQUEST_CAPACITY: usize = 1;

/// 单帧门控评估结果
#[derive(Debug, Clone, Copy)]
pub struct GateOutcome {
    /// 本帧决策
    pub decision: MotionGateDecision,
    /// 该路累计未参与门控判定的帧数（含本帧）
    pub bypassed_frames: u64,
}

/// 门控评估请求
struct GateRequest {
    frame: FrameRef,
    timestamp_ms: i64,
    /// 规则版本变更时随帧下发；未变更为 `None`，避免逐帧克隆规则
    rules: Option<Arc<Vec<DetectionRule>>>,
    reply: oneshot::Sender<GateOutcome>,
}

/// 每路相机一个的运动门控 OS Worker
pub struct MotionGateWorker {
    /// `None` 表示线程创建失败，本路门控已降级为保守放行
    tx: Option<mpsc::Sender<GateRequest>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// 线程退出信号：停机时用它做有界等待，避免轮询睡眠
    exit_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    /// 降级位：线程启动失败、线程退出或评估断路超时后置位，后续帧不再提交
    degraded: Arc<AtomicBool>,
    camera_id: String,
}

impl std::fmt::Debug for MotionGateWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MotionGateWorker")
            .field("camera", &self.camera_id)
            .field("is_alive", &self.is_alive())
            .finish()
    }
}

impl MotionGateWorker {
    /// 启动门控 Worker，`gate`（含常驻缩略图硬件上下文）移交该线程独占。
    ///
    /// 线程创建失败不 panic：内部置降级位，调用方只有一条统一路径——`evaluate` 返回 `None`，
    /// 由调用方保守放行并计入 `frames_gate_bypassed`，绝不静默伪装成「门控正在工作」。
    pub fn spawn(gate: MotionGate, camera_id: impl Into<String>) -> Self {
        let camera_id = camera_id.into();
        let (tx, rx) = mpsc::channel(GATE_REQUEST_CAPACITY);
        let degraded = Arc::new(AtomicBool::new(false));
        let thread_degraded = Arc::clone(&degraded);
        let (exit_tx, exit_rx) = std::sync::mpsc::channel();

        let spawned = std::thread::Builder::new()
            .name(format!("gate-{camera_id}"))
            .spawn(move || run(gate, rx, thread_degraded, ExitSignal(exit_tx)));

        match spawned {
            Ok(thread) => Self {
                tx: Some(tx),
                thread: Some(thread),
                exit_rx: Some(std::sync::Mutex::new(exit_rx)),
                degraded,
                camera_id,
            },
            Err(error) => {
                tracing::error!(
                    camera = %camera_id,
                    error = %error,
                    "运动门控工作线程创建失败，本路门控降级为保守放行"
                );
                degraded.store(true, Ordering::Relaxed);
                Self {
                    tx: None,
                    thread: None,
                    exit_rx: None,
                    degraded,
                    camera_id,
                }
            }
        }
    }

    /// 提交一帧并等待决策。
    ///
    /// 返回 `None` 表示门控不可用（线程创建失败/已退出、通道拥塞或评估断路超时）：
    /// 调用方必须保守放行该帧并计入绕过计数。
    pub async fn evaluate(
        &self,
        frame: FrameRef,
        timestamp_ms: i64,
        rules: Option<Arc<Vec<DetectionRule>>>,
    ) -> Option<GateOutcome> {
        if self.degraded.load(Ordering::Relaxed) {
            return None;
        }
        let tx = self.tx.as_ref()?;
        let (reply, reply_rx) = oneshot::channel();
        if let Err(error) = tx.try_send(GateRequest {
            frame,
            timestamp_ms,
            rules,
            reply,
        }) {
            tracing::debug!(camera = %self.camera_id, %error, "运动门控请求通道不可用");
            return None;
        }

        match tokio::time::timeout(GATE_EVAL_TIMEOUT, reply_rx).await {
            Ok(Ok(outcome)) => Some(outcome),
            // 工作线程在应答前退出：置降级位前不留半可用状态
            Ok(Err(_)) => {
                self.degraded.store(true, Ordering::Relaxed);
                tracing::error!(camera = %self.camera_id, "运动门控工作线程已退出，本路门控降级为保守放行");
                None
            }
            Err(_) => {
                self.degraded.store(true, Ordering::Relaxed);
                tracing::error!(
                    camera = %self.camera_id,
                    timeout_ms = GATE_EVAL_TIMEOUT.as_millis() as u64,
                    "运动门控评估断路超时，本路门控降级为保守放行（不再逐帧提交硬件工作）"
                );
                None
            }
        }
    }

    /// 门控 Worker 是否仍在服务（未降级）
    #[inline]
    pub fn is_alive(&self) -> bool {
        !self.degraded.load(Ordering::Relaxed)
    }

    /// 相机标识（仅用于诊断与日志归属）
    #[inline]
    pub fn camera_id(&self) -> &str {
        &self.camera_id
    }
}

impl Drop for MotionGateWorker {
    fn drop(&mut self) {
        // 关闭请求通道：工作线程处理完在途帧后从阻塞接收返回
        self.tx = None;
        self.degraded.store(true, Ordering::Relaxed);

        let (Some(thread), Some(exit_rx)) = (self.thread.take(), self.exit_rx.take()) else {
            return;
        };
        // 调用方可能是 Tokio Worker（解码任务结束或被取消时的局部变量析构），
        // 而有界等待会触及驱动挂死路径（最长 500ms），因此交给瞬态看护线程回收，
        // 绝不在异步执行器上阻塞。
        let camera_id = std::mem::take(&mut self.camera_id);
        let thread_name = format!("gate-reap-{camera_id}");
        let reaper_camera = camera_id.clone();
        let spawned = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                // `Disconnected` 说明发送端已随线程销毁，同样视为已退出
                let exited = exit_rx.lock().is_ok_and(|rx| {
                    matches!(
                        rx.recv_timeout(GATE_WORKER_SHUTDOWN_TIMEOUT),
                        Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                    )
                });
                if exited {
                    let _ = thread.join();
                    tracing::debug!(camera = %reaper_camera, "运动门控工作线程已优雅退出并回收");
                } else {
                    // 硬件驱动挂死：不无期限 `join`，交出句柄由系统回收，避免拖死控制面
                    tracing::error!(
                        camera = %reaper_camera,
                        timeout_ms = GATE_WORKER_SHUTDOWN_TIMEOUT.as_millis() as u64,
                        "运动门控工作线程停机超时（疑似硬件驱动内核调用挂起），隔离句柄受控保活"
                    );
                }
            });
        if let Err(error) = spawned {
            tracing::error!(
                camera = %camera_id,
                error = %error,
                "门控停机看护线程创建失败，已放弃回收门控工作线程"
            );
        }
    }
}

/// 线程退出信号（RAII：正常返回与 panic 展开都会触发，保证停机方不会空等）
struct ExitSignal(std::sync::mpsc::Sender<()>);

impl Drop for ExitSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// 工作线程主体：独占门控与常驻硬件上下文，逐帧串行评估
fn run(
    mut gate: MotionGate,
    mut rx: mpsc::Receiver<GateRequest>,
    degraded: Arc<AtomicBool>,
    _exit: ExitSignal,
) {
    while let Some(request) = rx.blocking_recv() {
        if let Some(rules) = request.rules.as_deref() {
            gate.update_rules(rules);
        }
        let decision = gate.evaluate_frame(&request.frame, request.timestamp_ms);
        // 应答发送失败只说明调用方已断路；门控状态仍按帧推进，保证参考帧链连续
        let _ = request.reply.send(GateOutcome {
            decision,
            bypassed_frames: gate.bypassed_frames(),
        });
    }
    degraded.store(true, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use types::{FrameHandle, MotionGateConfig, PixelFormat, StrideInfo};

    const CAMERA: &str = "cam_gate_worker";

    fn host_frame(base: u8, block: Option<u8>, timestamp_ms: i64) -> FrameRef {
        let mut bytes = vec![base; 64 * 64 * 3 / 2];
        if let Some(block_luma) = block {
            for y in 8..24 {
                for x in 8..24 {
                    bytes[y * 64 + x] = block_luma;
                }
            }
        }
        FrameRef::new(
            CAMERA.into(),
            timestamp_ms,
            64,
            64,
            StrideInfo::new(64, 64),
            PixelFormat::Nv12,
            FrameHandle::Host(bytes.into()),
        )
    }

    fn gate_with(contour_area: u32) -> MotionGate {
        MotionGate::new(MotionGateConfig {
            enabled: true,
            threshold: 25,
            contour_area,
            keepalive_interval_ms: 60_000,
            motion_hold_frames: 0,
        })
        .with_camera_id(CAMERA)
    }

    #[tokio::test]
    async fn test_worker_returns_ordered_decisions_off_tokio_worker() {
        let worker = MotionGateWorker::spawn(gate_with(16), CAMERA);
        assert!(worker.is_alive());

        // 首帧：无参考帧 → 保活放行
        let first = worker
            .evaluate(host_frame(100, None, 1000), 1000, None)
            .await
            .expect("门控 Worker 必须返回决策");
        assert!(!first.decision.should_skip);
        assert!(first.decision.is_keepalive);
        assert_eq!(first.bypassed_frames, 0, "Host 帧必须真正进入门控判定");

        // 第二帧静止：必须跳过推理
        let second = worker
            .evaluate(host_frame(100, None, 1040), 1040, None)
            .await
            .expect("门控 Worker 必须返回决策");
        assert!(second.decision.should_skip, "静止帧必须被门控跳过");

        // 局部运动（避开 85% 场景瞬变抑制）：必须放行
        let third = worker
            .evaluate(host_frame(100, Some(240), 1080), 1080, None)
            .await
            .expect("门控 Worker 必须返回决策");
        assert!(!third.decision.should_skip, "画面变化必须放行推理");
    }

    #[tokio::test]
    async fn test_worker_applies_rules_shipped_with_frame() {
        let worker = MotionGateWorker::spawn(gate_with(16), CAMERA);
        // 左上 32x32 遮罩：遮罩区内 16x16 的亮度跳变必须被抑制，清空规则后必须重新触发
        let half_mask = Arc::new(vec![DetectionRule {
            role: types::DetectionRuleRole::Mask,
            line_direction: types::DetectionLineDirection::Both,
            points: vec![
                types::DetectionPoint { x: 0.0, y: 0.0 },
                types::DetectionPoint { x: 0.5, y: 0.0 },
                types::DetectionPoint { x: 0.5, y: 0.5 },
                types::DetectionPoint { x: 0.0, y: 0.5 },
            ],
        }]);

        let first = worker
            .evaluate(
                host_frame(100, None, 1000),
                1000,
                Some(Arc::clone(&half_mask)),
            )
            .await
            .expect("首帧必须返回决策");
        assert!(first.decision.is_keepalive);

        let masked = worker
            .evaluate(host_frame(100, Some(240), 1040), 1040, None)
            .await
            .expect("第二帧必须返回决策");
        assert!(
            masked.decision.should_skip,
            "随帧下发的 Mask 规则必须在该 Worker 内生效"
        );
        assert_eq!(masked.decision.motion_score, 0.0);

        // 清空规则后同一块跳变必须重新触发运动
        let cleared = worker
            .evaluate(
                host_frame(100, Some(240), 1080),
                1080,
                Some(Arc::new(Vec::new())),
            )
            .await
            .expect("第三帧必须返回决策");
        assert!(
            !cleared.decision.should_skip,
            "规则清空后必须重新按运动放行"
        );
    }

    #[tokio::test]
    async fn test_worker_stops_within_bounded_time() {
        let worker = MotionGateWorker::spawn(gate_with(16), CAMERA);
        // 线程退出时会把降级位置位，用它作为“线程真的停了”的观测点
        let exited = Arc::clone(&worker.degraded);

        // 1. 析构不得在异步侧做有界等待（有界 `join` 由看护线程完成）
        let started = Instant::now();
        drop(worker);
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "门控 Worker 析构不得阻塞调用方（实测 {:?}）",
            started.elapsed()
        );

        // 2. 但工作线程必须在有界时间内真正退出
        let deadline = Instant::now() + GATE_WORKER_SHUTDOWN_TIMEOUT;
        while !exited.load(Ordering::Relaxed) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            exited.load(Ordering::Relaxed),
            "门控工作线程必须在有界时间内停机"
        );
    }
}
