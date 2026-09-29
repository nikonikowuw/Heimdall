/** 内部渲染分辨率缩放的上下限（在设备像素比折算后的附加系数） */
const PERF_SCALE_MIN = 0.65
const PERF_SCALE_MAX = 1

/** 判决帧率线：低于该值视为低算力终端，高于该值视为有余量 */
const LOW_FPS_THRESHOLD = 24
/** 低帧率时每步下调幅度 */
const DEGRADE_STEP = 0.15
/** 有余量时每步回升幅度：下调要果断，回升要保守 */
const RECOVER_STEP = 0.1
/**
 * 升档安全系数：升档后像素量约变为 (next/current)²，只有按该比例折算的帧率
 * 仍高于判决线的该倍数时才允许回升，避免在性能悬崖两侧反复横跳。
 */
const RECOVERY_MARGIN = 1.5

const round2 = (value: number): number => Math.round(value * 100) / 100

/**
 * 依据本秒实测帧率计算下一档内部渲染缩放；纯函数，便于用边界值单测钉住策略。
 *
 * - 帧率低于判决线：下调一档，不低于 {@link PERF_SCALE_MIN}；
 * - 帧率高于判决线：仅在「升档折算后仍有余量」时回升一档，否则维持（滞回区）；
 * - 帧率不可测（后台标签页、降级为静态渲染）：维持原档。
 *
 * 调用方负责在档位变化时重建绘制缓冲；本函数不触碰任何 WebGL 状态。
 */
export function nextPerfScale(current: number, fps: number): number {
  if (fps <= 0) {
    return current
  }

  if (fps < LOW_FPS_THRESHOLD) {
    return Math.max(PERF_SCALE_MIN, round2(current - DEGRADE_STEP))
  }

  const candidate = Math.min(PERF_SCALE_MAX, round2(current + RECOVER_STEP))
  if (candidate === current) {
    return current
  }

  const projectedFps = fps / (candidate / current) ** 2
  return projectedFps >= LOW_FPS_THRESHOLD * RECOVERY_MARGIN ? candidate : current
}
