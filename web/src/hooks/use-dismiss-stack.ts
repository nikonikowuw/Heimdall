import { useEffect, useId, useRef } from 'react'

export interface DismissOptions {
  /** 是否禁用 ESC 响应（如正在提交、删除或加载中） */
  disabled?: boolean
  /** 优先级，数值越高越优先被弹栈关闭，同优先级按后入先出 (LIFO)，默认 0 */
  priority?: number
  /** 是否在浮层打开时自动锁定页面 body 滚动，默认 true */
  lockScroll?: boolean
}

export interface DismissEntry {
  id: string
  getOnDismiss: () => () => void
  getDisabled: () => boolean
  priority: number
  lockScroll: boolean
}

// 模块级浮层栈与状态
const dismissStack: DismissEntry[] = []
let globalListenerAttached = false
let previousOverflow: string | null = null

export function updateBodyScrollLock() {
  if (typeof document === 'undefined' || !document.body) return
  const shouldLock = dismissStack.some((entry) => entry.lockScroll)

  if (shouldLock) {
    if (previousOverflow === null) {
      previousOverflow = document.body.style.overflow
      document.body.style.overflow = 'hidden'
    }
  } else if (previousOverflow !== null) {
    document.body.style.overflow = previousOverflow
    previousOverflow = null
  }
}

/**
 * 触发 ESC 键分发逻辑（返回是否被消费拦截）
 */
export function handleGlobalEscapeKeyDown(e?: {
  key?: string
  preventDefault?: () => void
  stopPropagation?: () => void
}): boolean {
  if (e?.key && e.key !== 'Escape') return false

  // 1. 如果浮层栈中有激活的浮层，按 LIFO + priority 寻找最顶层可响应项
  if (dismissStack.length > 0) {
    let maxPriority = -Infinity
    for (const entry of dismissStack) {
      if (entry.priority > maxPriority) {
        maxPriority = entry.priority
      }
    }

    // 从栈顶往栈底查找最高优先级的项
    for (let i = dismissStack.length - 1; i >= 0; i--) {
      const entry = dismissStack[i]
      if (entry.priority === maxPriority) {
        e?.preventDefault?.()
        e?.stopPropagation?.()
        if (!entry.getDisabled()) {
          entry.getOnDismiss()()
        }
        return true
      }
    }

    e?.preventDefault?.()
    e?.stopPropagation?.()
    return true
  }

  // 2. 栈为空：如果当前焦点在输入框/可编辑元素上，按 Esc 自动 blur 释放焦点
  if (typeof document !== 'undefined') {
    const activeEl = document.activeElement
    if (
      activeEl instanceof HTMLInputElement ||
      activeEl instanceof HTMLTextAreaElement ||
      (activeEl instanceof HTMLElement && activeEl.isContentEditable)
    ) {
      e?.preventDefault?.()
      activeEl.blur()
      return true
    }
  }

  return false
}

export function registerDismissEntry(entry: DismissEntry): () => void {
  dismissStack.push(entry)
  updateBodyScrollLock()
  return () => {
    const idx = dismissStack.findIndex((item) => item.id === entry.id)
    if (idx !== -1) {
      dismissStack.splice(idx, 1)
      updateBodyScrollLock()
    }
  }
}

function ensureGlobalListener() {
  if (globalListenerAttached || typeof window === 'undefined') return
  window.addEventListener('keydown', handleGlobalEscapeKeyDown, true)
  globalListenerAttached = true
}

/**
 * 检查当前是否有浮层正在打开
 */
export function isAnyModalOpen(): boolean {
  return dismissStack.length > 0
}

/**
 * 获取当前浮层栈长度（供测试与调试）
 */
export function getDismissStackLength(): number {
  return dismissStack.length
}

/**
 * 浮层与弹窗关闭栈 Hook
 * 遵循 LIFO (后入先出) 响应 ESC，杜绝嵌套弹窗穿透关闭外层；
 * 自动管理 body scroll 锁定并在组件卸载或关闭时安全清理。
 */
export function useDismissStack(
  isOpen: boolean,
  onDismiss: () => void,
  options: DismissOptions = {},
): void {
  const { disabled = false, priority = 0, lockScroll = true } = options
  const id = useId()

  const onDismissRef = useRef(onDismiss)
  onDismissRef.current = onDismiss

  const disabledRef = useRef(disabled)
  disabledRef.current = disabled

  useEffect(() => {
    ensureGlobalListener()

    if (!isOpen) return

    return registerDismissEntry({
      id,
      getOnDismiss: () => onDismissRef.current,
      getDisabled: () => disabledRef.current,
      priority,
      lockScroll,
    })
  }, [isOpen, id, priority, lockScroll])
}
