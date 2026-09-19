import React from 'react'
import { create } from 'zustand'

export type ToastType = 'success' | 'error' | 'warning' | 'info'

export interface ToastAction {
  label: string
  onClick: (e: React.MouseEvent<HTMLButtonElement>) => void
  primary?: boolean
}

export interface ToastItem {
  id: string
  type: ToastType
  title?: React.ReactNode
  message: React.ReactNode
  /** 工业 HUD 分类标识（如 "设备探活"、"任务调度"、"人员底库"） */
  category?: string
  /** 停留时间（毫秒），默认为 4000。为 0 或 <= 0 时不自动消退 */
  duration: number
  /** 自定义操作按键 */
  action?: ToastAction
  /** 创建时间戳（UTC 毫秒） */
  createdAt: number
}

export interface ToastInput {
  id?: string
  type?: ToastType
  title?: React.ReactNode
  message: React.ReactNode
  category?: string
  duration?: number
  action?: ToastAction
}

interface ToastState {
  toasts: ToastItem[]
  addToast: (input: ToastInput) => string
  dismissToast: (id: string) => void
  clearToasts: () => void
}

/** 屏幕上允许同时停留的最大 Toast 数量，超量自动淘汰最早的项，杜绝无界堆叠 */
export const MAX_TOASTS = 4
export const DEFAULT_TOAST_DURATION = 4000

export const useToastStore = create<ToastState>((set) => ({
  toasts: [],

  addToast: (input) => {
    const id = input.id || `toast-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`
    const duration = typeof input.duration === 'number' ? input.duration : DEFAULT_TOAST_DURATION
    const type = input.type || 'info'

    const newItem: ToastItem = {
      id,
      type,
      title: input.title,
      message: input.message,
      category: input.category,
      duration,
      action: input.action,
      createdAt: Date.now(),
    }

    set((state) => {
      // 过滤掉同 ID 冲突项，并将新通知排在最前面
      const filtered = state.toasts.filter((item) => item.id !== id)
      const nextToasts = [newItem, ...filtered]
      // 维持有界队列
      if (nextToasts.length > MAX_TOASTS) {
        return { toasts: nextToasts.slice(0, MAX_TOASTS) }
      }
      return { toasts: nextToasts }
    })

    return id
  },

  dismissToast: (id) => {
    set((state) => ({
      toasts: state.toasts.filter((item) => item.id !== id),
    }))
  },

  clearToasts: () => {
    set({ toasts: [] })
  },
}))

type ToastOptions = Omit<ToastInput, 'message' | 'type'>

function isToastInput(value: unknown): value is ToastInput {
  return (
    typeof value === 'object' &&
    value !== null &&
    !React.isValidElement(value) &&
    'message' in (value as object)
  )
}

function normalizeInput(
  type: ToastType,
  messageOrOptions: React.ReactNode | ToastInput,
  extraOptions?: ToastOptions,
): ToastInput {
  if (isToastInput(messageOrOptions)) {
    return {
      ...messageOrOptions,
      ...extraOptions,
      type,
    }
  }

  return {
    ...extraOptions,
    type,
    message: messageOrOptions,
  }
}

/**
 * 全局即插即用的 Toast 呼叫句柄。
 *
 * 既可在 React 组件内部使用，也可在常规异步函数、API 拦截器与 WebSocket 回调中调用。
 */
export const toast = {
  show: (input: ToastInput): string => {
    return useToastStore.getState().addToast(input)
  },

  success: (messageOrOptions: React.ReactNode | ToastInput, options?: ToastOptions): string => {
    return useToastStore.getState().addToast(normalizeInput('success', messageOrOptions, options))
  },

  error: (messageOrOptions: React.ReactNode | ToastInput, options?: ToastOptions): string => {
    return useToastStore.getState().addToast(normalizeInput('error', messageOrOptions, options))
  },

  warning: (messageOrOptions: React.ReactNode | ToastInput, options?: ToastOptions): string => {
    return useToastStore.getState().addToast(normalizeInput('warning', messageOrOptions, options))
  },

  info: (messageOrOptions: React.ReactNode | ToastInput, options?: ToastOptions): string => {
    return useToastStore.getState().addToast(normalizeInput('info', messageOrOptions, options))
  },

  dismiss: (id: string): void => {
    useToastStore.getState().dismissToast(id)
  },

  clear: (): void => {
    useToastStore.getState().clearToasts()
  },
}
