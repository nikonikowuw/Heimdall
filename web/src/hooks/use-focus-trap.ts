import { useEffect, type RefObject } from 'react'

/**
 * 可聚焦元素选择器：与 `ModalOverlay` 及原生 Tab 顺序保持一致的判定集合。
 * 导出以便测试与调用方复用同一份定义，避免各处复制后漂移。
 */
export const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ')

/**
 * 焦点陷阱：把 Tab 循环限制在容器内，并在浮层关闭后把焦点归还给打开前的元素。
 *
 * ## 为什么必须收敛成共享实现
 *
 * 仓库此前在 `ModalOverlay`、用户菜单、确认弹窗三处各写了一套焦点逻辑：
 * 手写集合的选择器不一致、`previouslyFocused` 归还时机不同、有的漏了「焦点在容器外」
 * 的兜底分支。这类横切关注点一旦分散，每新增一个浮层就多一次手写机会，
 * 于是出现「声明了 `aria-modal="true"` 却没有焦点陷阱」的静默可访问性缺口。
 *
 * ## 与浮层栈的关系
 *
 * 本 hook 只负责**焦点几何**，不处理 ESC / Enter（那是
 * [use-dismiss-stack](./use-dismiss-stack.ts) 的职责）。两者组合使用：
 * `useDismissStack` 管键盘语义与 body 滚动锁，本 hook 管焦点约束与归还。
 *
 * ## 判定说明
 *
 * 按 `tagName` / 属性结构判定而非 `instanceof HTMLElement` 子类，
 * 因此可在无真实 DOM 全局对象的测试环境下运行（与 `use-dismiss-stack` 一致）。
 */
export function useFocusTrap(isActive: boolean, containerRef: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    if (!isActive) return
    const container = containerRef.current
    if (!container) return

    const previouslyFocused =
      typeof document !== 'undefined' && document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null

    // 首个焦点目标优先取显式声明的 `[data-autofocus]`，其次容器内第一个可聚焦元素，
    // 都没有时退到容器自身（tabIndex={-1}），避免焦点留在浮层背后
    const initialTarget =
      container.querySelector<HTMLElement>('[data-autofocus]') ??
      container.querySelector<HTMLElement>(FOCUSABLE_SELECTOR) ??
      container
    initialTarget.focus({ preventScroll: true })

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return

      // 仅在可见元素间循环：`offsetParent === null` 表示被隐藏（含 `display: none`
      // 的折叠分区），把隐藏元素算进首尾会让 Tab 停在不可见控件上。
      const focusable = [...container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)].filter(
        (element) => element.offsetParent !== null || element === document.activeElement,
      )

      if (focusable.length === 0) {
        // 容器内无可用焦点目标时，把焦点留在容器自身，避免 Tab 逃逸到浮层背后
        event.preventDefault()
        container.focus()
        return
      }

      const first = focusable[0]
      const last = focusable[focusable.length - 1]
      const active = typeof document !== 'undefined' ? document.activeElement : null

      if (event.shiftKey && (active === first || !container.contains(active))) {
        event.preventDefault()
        last.focus()
      } else if (!event.shiftKey && (active === last || !container.contains(active))) {
        event.preventDefault()
        first.focus()
      }
    }

    container.addEventListener('keydown', handleKeyDown)
    return () => {
      container.removeEventListener('keydown', handleKeyDown)
      previouslyFocused?.focus({ preventScroll: true })
    }
  }, [isActive, containerRef])
}
