import type { TrackedBBox } from '../types'
import { TrackRingBuffer } from './trackRingBuffer'

export type TrackListener = (tracks: TrackedBBox[], pts?: number) => void

interface TrackSnapshot {
  tracks: TrackedBBox[]
  updatedAt: number
  pts?: number
}

/** 航迹快照有效期 (1500ms)，超过该时间未更新视作陈旧历史，新订阅者不会回放陈旧快照 */
export const TRACK_SNAPSHOT_TTL_MS = 1500

/**
 * 实时航迹内存订阅总线 (零 React 响应式开销)
 *
 * 针对 10~15 FPS 高频检测框更新与 PTS 时空对齐设计：
 * 1. 数据保存在内存 Map 与每路摄像头的轻量 TrackRingBuffer 中，完全绕开 React useState，杜绝高频 DOM 重排；
 * 2. 采用针对单个摄像头的微观察者模式，播放器挂载时直接监听，卸载时自动清理；
 * 3. 驱动 LivePlayer Canvas 2D 离屏渲染循环在下一帧直接按当前视频画面的 PTS 对齐绘制；
 * 4. 内置快照 TTL 与容差窗口防滞留机制，避免拉流重建或页面切换时回放数秒前的陈旧幽灵框。
 */
class TrackStore {
  private tracksByCamera = new Map<string, TrackSnapshot>()
  private ringBuffersByCamera = new Map<string, TrackRingBuffer>()
  private listeners = new Map<string, Set<TrackListener>>()

  /**
   * 写入某路摄像头的最新航迹并通知该摄像头的活跃监听器
   *
   * @param cameraId 摄像头 ID
   * @param tracks 目标航迹列表
   * @param pts 13 位源帧 UTC Unix 毫秒时间戳 (可选)
   */
  setTracks(cameraId: string, tracks: TrackedBBox[], pts?: number): void {
    this.tracksByCamera.set(cameraId, {
      tracks,
      updatedAt: Date.now(),
      pts,
    })

    if (pts && Number.isFinite(pts) && pts > 0) {
      let ring = this.ringBuffersByCamera.get(cameraId)
      if (!ring) {
        ring = new TrackRingBuffer()
        this.ringBuffersByCamera.set(cameraId, ring)
      }
      ring.push(pts, tracks)
    }

    const subs = this.listeners.get(cameraId)
    if (subs) {
      for (const listener of subs) {
        if (pts !== undefined) {
          listener(tracks, pts)
        } else {
          listener(tracks)
        }
      }
    }
  }

  /**
   * 获取某路摄像头的航迹数据
   *
   * @param cameraId 摄像头 ID
   * @param currentVideoPts 当前正在渲染的视频帧绝对毫秒时间戳 (可选)。
   *                        若传入有效时间戳，优先通过环形缓冲队列做二分查找时空对齐；
   *                        若未传入或未命中，平滑降级至最新快照 (TTL 内)。
   * @param maxToleranceMs 最大允许时间差 (默认 150ms)
   */
  getTracks(
    cameraId: string,
    currentVideoPts?: number | null,
    maxToleranceMs?: number,
  ): TrackedBBox[] {
    // 1. 若提供了视频帧 PTS，优先尝试通过环形队列毫秒对齐
    if (currentVideoPts != null && Number.isFinite(currentVideoPts) && currentVideoPts > 0) {
      const ring = this.ringBuffersByCamera.get(cameraId)
      if (ring && ring.size > 0) {
        return ring.lookup(currentVideoPts, maxToleranceMs)
      }
    }

    // 2. 降级回退：读取最新的有效快照
    const snapshot = this.tracksByCamera.get(cameraId)
    if (!snapshot || Date.now() - snapshot.updatedAt > TRACK_SNAPSHOT_TTL_MS) {
      return []
    }
    return snapshot.tracks
  }

  /**
   * 订阅某路摄像头的航迹更新流
   *
   * @returns 取消订阅的回调函数
   */
  subscribe(cameraId: string, listener: TrackListener): () => void {
    let set = this.listeners.get(cameraId)
    if (!set) {
      set = new Set()
      this.listeners.set(cameraId, set)
    }
    set.add(listener)

    // 仅在当前快照有效且未超时过期时立即触发回放，防止重新挂载时闪现陈旧历史框
    const cached = this.getTracks(cameraId)
    if (cached.length > 0) {
      listener(cached)
    }

    return () => {
      const activeSet = this.listeners.get(cameraId)
      if (activeSet) {
        activeSet.delete(listener)
        if (activeSet.size === 0) {
          this.listeners.delete(cameraId)
        }
      }
    }
  }

  /**
   * 清除某路或全部摄像头的航迹缓存与环形队列
   */
  clear(cameraId?: string): void {
    if (cameraId) {
      this.tracksByCamera.delete(cameraId)
      this.ringBuffersByCamera.delete(cameraId)
      this.listeners.get(cameraId)?.forEach((listener) => listener([]))
    } else {
      this.tracksByCamera.clear()
      this.ringBuffersByCamera.clear()
      for (const subs of this.listeners.values()) {
        subs.forEach((listener) => listener([]))
      }
    }
  }
}

export const trackStore = new TrackStore()
