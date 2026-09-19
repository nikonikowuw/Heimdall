import { useEffect } from 'react'
import type { NavTab } from '@/types'
import { isAnyModalOpen } from './use-dismiss-stack'

export interface GlobalShortcutsOptions {
  currentTab: NavTab
  onSelectTab: (tab: NavTab) => void
  onToggleTheme: () => void
  onOpenShortcutsHelp: () => void
  enabled?: boolean
}

export const TAB_SHORTCUT_MAP: Record<string, NavTab> = {
  '1': 'live',
  '2': 'cameras',
  '3': 'tasks',
  '4': 'algorithms',
  '5': 'personnel',
  '6': 'alarms',
  '7': 'oplog',
  '8': 'system',
}

function isEditableElement(element: Element | null): boolean {
  if (!element) return false
  if (
    element instanceof HTMLInputElement ||
    element instanceof HTMLTextAreaElement ||
    element instanceof HTMLSelectElement
  ) {
    return true
  }
  return element instanceof HTMLElement && element.isContentEditable
}

/**
 * 全局按键处理核心逻辑（可独立测试）
 */
export function handleGlobalShortcutEvent(
  e: {
    key: string
    code?: string
    altKey: boolean
    ctrlKey: boolean
    metaKey: boolean
    shiftKey: boolean
    target?: EventTarget | null
    preventDefault?: () => void
    stopPropagation?: () => void
  },
  options: {
    onSelectTab: (tab: NavTab) => void
    onToggleTheme: () => void
    onOpenShortcutsHelp: () => void
    isModalOpen?: () => boolean
    focusSearchInput?: () => boolean
  },
): boolean {
  // 不干预浏览器的 Cmd/Ctrl 复合快捷键（如 Cmd+R, Cmd+1 等标签页切换）
  if (e.metaKey || e.ctrlKey) {
    return false
  }

  const target =
    (e.target as Element | null) ??
    (typeof document !== 'undefined' ? document.activeElement : null)
  const inEditable = isEditableElement(target)
  const modalOpen = options.isModalOpen ? options.isModalOpen() : isAnyModalOpen()

  // 1. 在可编辑输入框中：
  if (inEditable) {
    // 只有 Alt+数字 允许在输入状态下强制跳转 Tab，纯数字等其他单键严禁拦截输入
    if (e.altKey && Object.prototype.hasOwnProperty.call(TAB_SHORTCUT_MAP, e.key)) {
      const tab = TAB_SHORTCUT_MAP[e.key]
      if (tab) {
        e.preventDefault?.()
        e.stopPropagation?.()
        options.onSelectTab(tab)
        return true
      }
    }
    return false
  }

  // 2. 如果存在弹窗浮层，禁用全局单键快捷键，让用户专注当前弹窗
  if (modalOpen) {
    return false
  }

  // 3. 页面无弹窗且未在输入框时的快捷键处理：

  // 3.1 帮助面板：? 或 Shift + /
  if (e.key === '?' || (e.shiftKey && e.key === '/')) {
    e.preventDefault?.()
    e.stopPropagation?.()
    options.onOpenShortcutsHelp()
    return true
  }

  // 3.2 Tab 快速切换：1~8 或 Alt + 1~8
  if (Object.prototype.hasOwnProperty.call(TAB_SHORTCUT_MAP, e.key)) {
    const tab = TAB_SHORTCUT_MAP[e.key]
    if (tab) {
      e.preventDefault?.()
      e.stopPropagation?.()
      options.onSelectTab(tab)
      return true
    }
  }

  // 3.3 主题切换：'t' 或 'T' 或 Alt+T
  if (e.key === 't' || e.key === 'T') {
    e.preventDefault?.()
    e.stopPropagation?.()
    options.onToggleTheme()
    return true
  }

  // 3.4 快速聚焦搜索框：'/'
  if (e.key === '/' && !e.shiftKey) {
    const focused = options.focusSearchInput ? options.focusSearchInput() : tryFocusSearchInput()
    if (focused) {
      e.preventDefault?.()
      e.stopPropagation?.()
      return true
    }
  }

  return false
}

function tryFocusSearchInput(): boolean {
  if (typeof document === 'undefined') return false
  const searchInput = document.querySelector<HTMLInputElement>(
    'input[type="search"], input[data-search="true"], input[placeholder*="搜索"], input[placeholder*="Search"], input[placeholder*="搜尋"]',
  )
  if (searchInput && !searchInput.disabled) {
    searchInput.focus()
    searchInput.select()
    return true
  }
  return false
}

/**
 * 全局键盘导航与快捷键 Hook
 */
export function useGlobalShortcuts({
  onSelectTab,
  onToggleTheme,
  onOpenShortcutsHelp,
  enabled = true,
}: GlobalShortcutsOptions): void {
  useEffect(() => {
    if (!enabled || typeof window === 'undefined') return

    const listener = (e: KeyboardEvent) => {
      handleGlobalShortcutEvent(e, {
        onSelectTab,
        onToggleTheme,
        onOpenShortcutsHelp,
      })
    }

    window.addEventListener('keydown', listener)
    return () => {
      window.removeEventListener('keydown', listener)
    }
  }, [enabled, onSelectTab, onToggleTheme, onOpenShortcutsHelp])
}
