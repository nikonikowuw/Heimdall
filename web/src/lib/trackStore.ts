import type { TrackedBBox } from '../types'

export type TrackListener = (tracks: TrackedBBox[]) => void

interface TrackSnapshot {
  tracks: TrackedBBox[]
  updatedAt: number
}

/** 航迹快照有效期 (1500ms)，超过该时间未更新视作陈旧历史，新订阅者不会回放陈旧快照 */
export const TRACK_SNAPSHOT_TTL_MS = 1500

/**
 * 实时航迹内存订阅总线 (零 React 响应式开销)
 *
 * 针对 10~15 FPS 高频检测框更新设计：
 * 1. 数据保存在内存 Map 中，完全绕开 React useState，杜绝高频 DOM 重排；
 * 2. 采用针对单个摄像头的微观察者模式，播放器挂载时直接监听，卸载时自动清理；
 * 3. 驱动 LivePlayer Canvas 2D 离屏渲染循环在下一帧直接绘制最新航迹；
 * 4. 内置快照 TTL 防滞留机制，避免拉流重建或页面切换时回放数分钟前的陈旧幽灵框。
 */
class TrackStore {
  private tracksByCamera = new Map<string, TrackSnapshot>()
  private listeners = new Map<string, Set<TrackListener>>()

  /**
   * 写入某路摄像头的最新航迹并通知该摄像头的活跃监听器
   */
  setTracks(cameraId: string, tracks: TrackedBBox[]): void {
    this.tracksByCamera.set(cameraId, {
      tracks,
      updatedAt: Date.now(),
    })
    const subs = this.listeners.get(cameraId)
    if (subs) {
      for (const listener of subs) {
        listener(tracks)
      }
    }
  }

  /**
   * 同步获取某路摄像头当前的最新航迹快照 (若已过期则返回空数组)
   */
  getTracks(cameraId: string): TrackedBBox[] {
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
   * 清除某路或全部摄像头的航迹缓存
   */
  clear(cameraId?: string): void {
    if (cameraId) {
      this.tracksByCamera.delete(cameraId)
      const subs = this.listeners.get(cameraId)
      if (subs) {
        for (const listener of subs) {
          listener([])
        }
      }
    } else {
      this.tracksByCamera.clear()
      for (const subs of this.listeners.values()) {
        for (const listener of subs) {
          listener([])
        }
      }
    }
  }
}

export const trackStore = new TrackStore()
