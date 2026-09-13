import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getDismissStackLength,
  handleGlobalEscapeKeyDown,
  isAnyModalOpen,
  registerDismissEntry,
} from './use-dismiss-stack'

describe('useDismissStack & Dismiss Manager', () => {
  const cleanups: (() => void)[] = []

  beforeEach(() => {
    while (cleanups.length > 0) {
      cleanups.pop()!()
    }
  })

  afterEach(() => {
    while (cleanups.length > 0) {
      cleanups.pop()!()
    }
  })

  it('should initially be empty', () => {
    expect(isAnyModalOpen()).toBe(false)
    expect(getDismissStackLength()).toBe(0)
  })

  it('should register and unregister entries cleanly', () => {
    const onDismiss = vi.fn()
    const unregister = registerDismissEntry({
      id: 'modal-1',
      getOnDismiss: () => onDismiss,
      getDisabled: () => false,
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unregister)

    expect(isAnyModalOpen()).toBe(true)
    expect(getDismissStackLength()).toBe(1)

    unregister()
    expect(isAnyModalOpen()).toBe(false)
    expect(getDismissStackLength()).toBe(0)
  })

  it('should pop and call top entry on Escape (LIFO order)', () => {
    const onDismiss1 = vi.fn()
    const onDismiss2 = vi.fn()

    const unreg1 = registerDismissEntry({
      id: 'modal-1',
      getOnDismiss: () => onDismiss1,
      getDisabled: () => false,
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unreg1)

    const unreg2 = registerDismissEntry({
      id: 'modal-2',
      getOnDismiss: () => onDismiss2,
      getDisabled: () => false,
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unreg2)

    expect(getDismissStackLength()).toBe(2)

    // 第一次按 Escape：应调用后打开的 modal-2，modal-1 不被调用
    const consumed = handleGlobalEscapeKeyDown({ key: 'Escape' })
    expect(consumed).toBe(true)
    expect(onDismiss2).toHaveBeenCalledTimes(1)
    expect(onDismiss1).not.toHaveBeenCalled()

    // 模拟 modal-2 响应后 unregister
    unreg2()

    // 第二次按 Escape：应调用 modal-1
    handleGlobalEscapeKeyDown({ key: 'Escape' })
    expect(onDismiss1).toHaveBeenCalledTimes(1)
  })

  it('should respect priority over registration order', () => {
    const onDismissLow = vi.fn()
    const onDismissHigh = vi.fn()

    // 先注册低优先级
    const unregLow = registerDismissEntry({
      id: 'drawer-low',
      getOnDismiss: () => onDismissLow,
      getDisabled: () => false,
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unregLow)

    // 注册高优先级（例如顶层图片灯箱）
    const unregHigh = registerDismissEntry({
      id: 'lightbox-high',
      getOnDismiss: () => onDismissHigh,
      getDisabled: () => false,
      priority: 10,
      lockScroll: false,
    })
    cleanups.push(unregHigh)

    handleGlobalEscapeKeyDown({ key: 'Escape' })
    expect(onDismissHigh).toHaveBeenCalledTimes(1)
    expect(onDismissLow).not.toHaveBeenCalled()
  })

  it('should not call disabled top entry but prevent pass-through', () => {
    const onDismissUnder = vi.fn()
    const onDismissTopDisabled = vi.fn()

    const unreg1 = registerDismissEntry({
      id: 'under',
      getOnDismiss: () => onDismissUnder,
      getDisabled: () => false,
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unreg1)

    const unreg2 = registerDismissEntry({
      id: 'top-submitting',
      getOnDismiss: () => onDismissTopDisabled,
      getDisabled: () => true, // 禁用中（如正在提交）
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unreg2)

    const consumed = handleGlobalEscapeKeyDown({ key: 'Escape' })
    expect(consumed).toBe(true)
    // 两者均不被调用，且事件被消费（不穿透到底层）
    expect(onDismissTopDisabled).not.toHaveBeenCalled()
    expect(onDismissUnder).not.toHaveBeenCalled()
  })

  it('should ignore non-Escape keys', () => {
    const onDismiss = vi.fn()
    const unreg = registerDismissEntry({
      id: 'test',
      getOnDismiss: () => onDismiss,
      getDisabled: () => false,
      priority: 0,
      lockScroll: false,
    })
    cleanups.push(unreg)

    const consumed = handleGlobalEscapeKeyDown({ key: 'Enter' })
    expect(consumed).toBe(false)
    expect(onDismiss).not.toHaveBeenCalled()
  })
})
