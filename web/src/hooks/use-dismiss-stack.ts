import { useEffect, useId, useRef } from 'react'

export interface DismissOptions {
  /** 是否禁用 ESC 响应（如正在提交、删除或加载中） */
  disabled?: boolean
  /** 优先级，数值越高越优先被弹栈关闭，同优先级按后入先出 (LIFO)，默认 0 */
  priority?: number
  /** 是否在浮层打开时自动锁定页面 body 滚动，默认 true */
  lockScroll?: boolean
  /**
   * Enter 确认回调。传入时该浮层成为栈顶后接管 Enter（确认类弹窗的键盘契约）。
   * 不传表示浮层不声明 Enter 语义，此时 Enter 不被拦截。
   */
  onConfirm?: () => void
}

export interface DismissEntry {
  id: string
  getOnDismiss: () => () => void
  getOnConfirm?: () => (() => void) | undefined
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

/** 按 LIFO + priority 解析当前最顶层可响应浮层 */
function topDismissEntry(): DismissEntry | null {
  let maxPriority = -Infinity
  for (const entry of dismissStack) {
    if (entry.priority > maxPriority) {
      maxPriority = entry.priority
    }
  }

  // 从栈顶往栈底查找最高优先级的项
  for (let i = dismissStack.length - 1; i >= 0; i--) {
    const entry = dismissStack[i]
    if (entry.priority === maxPriority) return entry
  }

  return null
}

/** 输入类控件标签：Enter 与 ESC 都必须避让，否则会吞掉用户正在进行的输入 */
const EDITABLE_TAGS = new Set(['INPUT', 'TEXTAREA'])

/**
 * 当前获得焦点的可编辑元素；无焦点或非输入控件时返回 null。
 * 按 tagName 结构判定而非 `instanceof`，因此可在无 HTMLElement 全局对象的测试环境下运行。
 */
function editableFocusTarget(): HTMLElement | null {
  if (typeof document === 'undefined') return null
  const activeElement = document.activeElement as HTMLElement | null
  if (!activeElement) return null

  const isEditable = EDITABLE_TAGS.has(activeElement.tagName) || activeElement.isContentEditable
  return isEditable ? activeElement : null
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
  const top = topDismissEntry()
  if (top) {
    e?.preventDefault?.()
    e?.stopPropagation?.()
    if (!top.getDisabled()) {
      top.getOnDismiss()()
    }
    return true
  }

  // 2. 栈为空：如果当前焦点在输入框/可编辑元素上，按 Esc 自动 blur 释放焦点
  const editable = editableFocusTarget()
  if (editable) {
    e?.preventDefault?.()
    editable.blur()
    return true
  }

  return false
}

/**
 * 触发 Enter 确认分发逻辑（返回是否被消费拦截）。
 *
 * 仅最顶层浮层可接管 Enter，且必须是它自己声明了 `onConfirm`；
 * 未声明的浮层不拦截，以免吞掉页面自身的 Enter 语义（列表项激活、搜索提交等）。
 */
export function handleGlobalConfirmKeyDown(e?: {
  key?: string
  preventDefault?: () => void
  stopPropagation?: () => void
}): boolean {
  if (e?.key && e.key !== 'Enter') return false

  const top = topDismissEntry()
  const onConfirm = top?.getOnConfirm?.()
  if (!top || !onConfirm) return false

  // 输入态下 Enter 属于用户输入行为，不得升级为确认；也不应拦截该按键
  if (editableFocusTarget()) return false

  e?.preventDefault?.()
  e?.stopPropagation?.()
  if (!top.getDisabled()) {
    onConfirm()
  }
  return true
}

function handleGlobalDismissKeyDown(e: KeyboardEvent): void {
  if (e.key === 'Enter') {
    handleGlobalConfirmKeyDown(e)
    return
  }
  handleGlobalEscapeKeyDown(e)
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
  window.addEventListener('keydown', handleGlobalDismissKeyDown, true)
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
  const { disabled = false, priority = 0, lockScroll = true, onConfirm } = options
  const id = useId()

  const onDismissRef = useRef(onDismiss)
  onDismissRef.current = onDismiss

  const onConfirmRef = useRef(onConfirm)
  onConfirmRef.current = onConfirm

  const disabledRef = useRef(disabled)
  disabledRef.current = disabled

  useEffect(() => {
    ensureGlobalListener()

    if (!isOpen) return

    return registerDismissEntry({
      id,
      getOnDismiss: () => onDismissRef.current,
      getOnConfirm: () => onConfirmRef.current,
      getDisabled: () => disabledRef.current,
      priority,
      lockScroll,
    })
  }, [isOpen, id, priority, lockScroll])
}
