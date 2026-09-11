import type { TrackedBBox } from '../types'

export interface TimedTrackFrame {
  pts: number // 13 位 UTC Unix 毫秒时间戳
  tracks: TrackedBBox[]
}

/** 环形队列默认容量 (存储最近 60 帧，按 15fps 约 4 秒历史) */
export const DEFAULT_RING_BUFFER_CAPACITY = 60

/** 允许的最大音视频/AI 元数据容差窗口 (默认 150ms) */
export const DEFAULT_MAX_TOLERANCE_MS = 150

/**
 * 基于源帧 PTS (毫秒时间戳) 的实时航迹环形缓冲队列
 *
 * 核心目标：
 * 1. 消除视频流播放缓冲与 AI 推理排队耗时不同步导致的「框跑在人前面」或「幽灵框滞后」时空撕裂问题；
 * 2. 在渲染循环中根据视频画面当前正在展示的真实 PTS，通过 O(log N) 二分查找提取最匹配的航迹；
 * 3. 固定上限有界环形存储，零垃圾回收抖动，单路摄像头内存占用 < 50KB。
 */
export class TrackRingBuffer {
  private buffer: TimedTrackFrame[] = []
  private readonly maxCapacity: number

  constructor(maxCapacity = DEFAULT_RING_BUFFER_CAPACITY) {
    this.maxCapacity = maxCapacity
  }

  /**
   * 写入带有源帧 PTS 的航迹数据
   * 自动保持按 PTS 严格升序排列，同源帧时间戳原地幂等更新，并维护容量上限
   */
  push(pts: number, tracks: TrackedBBox[]): void {
    if (!Number.isFinite(pts) || pts <= 0) return

    const last = this.buffer[this.buffer.length - 1]

    // 1. 同一源帧更新（如多算法实例聚合或重传）：原地更新，保持严格单调性
    if (last?.pts === pts) {
      last.tracks = tracks
      return
    }

    const frame: TimedTrackFrame = { pts, tracks }

    // 2. 正常时序递增追加 (O(1))
    if (!last || pts > last.pts) {
      this.buffer.push(frame)
    } else {
      // 3. 偶发网络乱序到达：二分查找升序插入位置或相同时间戳更新
      let low = 0
      let high = this.buffer.length - 1
      let insertIdx = this.buffer.length

      while (low <= high) {
        const mid = Math.floor((low + high) / 2)
        const midPts = this.buffer[mid].pts

        if (midPts === pts) {
          this.buffer[mid].tracks = tracks
          return
        }
        if (midPts > pts) {
          insertIdx = mid
          high = mid - 1
        } else {
          low = mid + 1
        }
      }
      this.buffer.splice(insertIdx, 0, frame)
    }

    // 维持有界容量上限
    if (this.buffer.length > this.maxCapacity) {
      this.buffer.shift()
    }
  }

  /**
   * 根据当前视频画面的 PTS 时间戳，二分查找最匹配的航迹数据
   *
   * @param currentVideoPts 当前视频画面的 13 位毫秒时间戳
   * @param maxToleranceMs 最大允许容忍时间误差 (默认 150ms)，超过上限视为画面与算法失步，返回空数组
   */
  lookup(currentVideoPts: number, maxToleranceMs = DEFAULT_MAX_TOLERANCE_MS): TrackedBBox[] {
    if (this.buffer.length === 0 || !Number.isFinite(currentVideoPts) || currentVideoPts <= 0) {
      return []
    }

    let low = 0
    let high = this.buffer.length - 1
    let bestMatch = this.buffer[0]
    let minDiff = Math.abs(bestMatch.pts - currentVideoPts)

    while (low <= high) {
      const mid = Math.floor((low + high) / 2)
      const frame = this.buffer[mid]
      const diff = Math.abs(frame.pts - currentVideoPts)

      if (diff < minDiff) {
        minDiff = diff
        bestMatch = frame
      }

      if (frame.pts === currentVideoPts) {
        return frame.tracks
      }
      if (frame.pts < currentVideoPts) {
        low = mid + 1
      } else {
        high = mid - 1
      }
    }

    // 容忍阈值防护：若时间差超出容忍范围（例如视频暂停或时间戳跳变），丢弃匹配避免绘制过时幽灵框
    return minDiff <= maxToleranceMs ? bestMatch.tracks : []
  }

  /**
   * 获取队列中最新的一组航迹 (无视频 PTS 时的降级方案)
   */
  getLatest(): TrackedBBox[] {
    return this.buffer[this.buffer.length - 1]?.tracks ?? []
  }

  /**
   * 清空缓冲区
   */
  clear(): void {
    this.buffer = []
  }

  /**
   * 当前缓冲区大小 (用于单元测试与调试)
   */
  get size(): number {
    return this.buffer.length
  }
}
