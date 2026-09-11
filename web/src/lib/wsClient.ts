import { useAuthStore } from '../stores/auth'
import { type CameraTracksPayload, WS_TOPICS } from '../types'
import { trackStore } from './trackStore'

export type WsEventHandler<T = unknown> = (payload: T, timestamp: number) => void

/**
 * 全局 WebSocket 事件客户端 (单例复用与指数退避重连)
 *
 * 规范遵循 (AGENTS.md):
 * 1. 全局唯一长连接复用，避免各页面各自创建 WebSocket 导致的连接与带宽浪费；
 * 2. 指数退避智能重连 (1s, 2s, 4s ... 上限 30s)；
 * 3. 高频实时航迹数据 (camera.tracks) 默认直通 trackStore，零 React 响应式开销；
 * 4. 业务事件 (camera.probe_updated, alarm.triggered 等) 通过观察者派发。
 */
class WsClient {
  private ws: WebSocket | null = null
  private reconnectAttempts = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private listeners = new Map<string, Set<WsEventHandler<unknown>>>()
  private isInitialized = false

  init(): void {
    if (this.isInitialized) return
    this.isInitialized = true

    // 默认全局订阅：实时航迹流直接流向 trackStore (零组件重排，支持源帧 PTS 时空对齐)
    this.subscribe<CameraTracksPayload>(WS_TOPICS.CAMERA_TRACKS, (payload) => {
      if (payload?.cameraId) {
        trackStore.setTracks(payload.cameraId, payload.tracks ?? [], payload.timestamp)
      }
    })

    // 监听认证状态变化
    useAuthStore.subscribe((state) => {
      if (state.isAuthenticated && state.token) {
        this.connect()
      } else {
        this.disconnect()
      }
    })

    // 若当前环境已登录，立即连接
    if (useAuthStore.getState().isAuthenticated) {
      this.connect()
    }
  }

  connect(): void {
    if (typeof window === 'undefined') return
    const token = useAuthStore.getState().token
    if (!token) return

    if (
      this.ws &&
      (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)
    ) {
      return
    }

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const host = window.location.host
    const wsUrl = `${protocol}//${host}/api/v1/ws/events?token=${encodeURIComponent(token)}`

    try {
      const socket = new WebSocket(wsUrl)
      this.ws = socket

      socket.onopen = () => {
        this.reconnectAttempts = 0
      }

      socket.onmessage = (e) => {
        let event: {
          topic?: string
          payload?: unknown
          timestamp?: number
        }
        try {
          event = JSON.parse(e.data) as {
            topic?: string
            payload?: unknown
            timestamp?: number
          }
        } catch {
          // 忽略非法非 JSON 帧
          return
        }

        if (event.topic) {
          this.dispatch(event.topic, event.payload, event.timestamp)
        }
      }

      socket.onclose = () => {
        this.ws = null
        this.scheduleReconnect()
      }

      socket.onerror = () => {
        if (this.ws) {
          this.ws.close()
        }
      }
    } catch {
      this.scheduleReconnect()
    }
  }

  private scheduleReconnect(): void {
    if (!useAuthStore.getState().isAuthenticated) return
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000)
    this.reconnectAttempts += 1
    this.reconnectTimer = setTimeout(() => {
      this.connect()
    }, delay)
  }

  disconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.reconnectAttempts = 0
    if (this.ws) {
      this.ws.close()
      this.ws = null
    }
  }

  /**
   * 向指定主题的本地观察者派发消息事件 (支持单测模拟与消息流转)
   */
  dispatch(topic: string, payload: unknown, timestamp = Date.now()): void {
    const subs = this.listeners.get(topic)
    if (!subs) return

    for (const handler of subs) {
      try {
        handler(payload, timestamp)
      } catch {
        // 隔离单个订阅者异常，防止影响其他监听器
      }
    }
  }

  subscribe<T = unknown>(topic: string, handler: WsEventHandler<T>): () => void {
    let set = this.listeners.get(topic)
    if (!set) {
      set = new Set()
      this.listeners.set(topic, set)
    }
    set.add(handler as WsEventHandler<unknown>)

    return () => {
      const activeSet = this.listeners.get(topic)
      if (activeSet) {
        activeSet.delete(handler as WsEventHandler<unknown>)
        if (activeSet.size === 0) {
          this.listeners.delete(topic)
        }
      }
    }
  }
}

export const wsClient = new WsClient()
