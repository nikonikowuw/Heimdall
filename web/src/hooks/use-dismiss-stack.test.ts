import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getDismissStackLength,
  handleGlobalConfirmKeyDown,
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
    vi.unstubAllGlobals()
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

  it('restores the original body overflow only after the last locked modal closes', () => {
    const body = { style: { overflow: 'clip' } }
    vi.stubGlobal('document', { body })

    const unregisterFirst = registerDismissEntry({
      id: 'scroll-lock-first',
      getOnDismiss: () => vi.fn(),
      getDisabled: () => false,
      priority: 0,
      lockScroll: true,
    })
    cleanups.push(unregisterFirst)

    const unregisterSecond = registerDismissEntry({
      id: 'scroll-lock-second',
      getOnDismiss: () => vi.fn(),
      getDisabled: () => false,
      priority: 0,
      lockScroll: true,
    })
    cleanups.push(unregisterSecond)

    expect(body.style.overflow).toBe('hidden')
    unregisterFirst()
    expect(body.style.overflow).toBe('hidden')
    unregisterSecond()
    expect(body.style.overflow).toBe('clip')
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

  describe('Enter confirm dispatch', () => {
    it('runs onConfirm for the top entry that declares it', () => {
      const onConfirm = vi.fn()
      const unreg = registerDismissEntry({
        id: 'confirmable',
        getOnDismiss: () => vi.fn(),
        getOnConfirm: () => onConfirm,
        getDisabled: () => false,
        priority: 0,
        lockScroll: false,
      })
      cleanups.push(unreg)

      expect(handleGlobalConfirmKeyDown({ key: 'Enter' })).toBe(true)
      expect(onConfirm).toHaveBeenCalledTimes(1)
    })

    it('does not intercept Enter when the top entry declares no confirm', () => {
      const unreg = registerDismissEntry({
        id: 'no-confirm',
        getOnDismiss: () => vi.fn(),
        getDisabled: () => false,
        priority: 0,
        lockScroll: false,
      })
      cleanups.push(unreg)

      // 未声明 onConfirm 的浮层不得吞掉页面自身的 Enter 语义
      expect(handleGlobalConfirmKeyDown({ key: 'Enter' })).toBe(false)
    })

    it('does not intercept Enter on an empty stack', () => {
      expect(handleGlobalConfirmKeyDown({ key: 'Enter' })).toBe(false)
    })

    it('routes Enter to the highest-priority entry, not the newest', () => {
      const onConfirmLow = vi.fn()
      const onConfirmHigh = vi.fn()
      cleanups.push(
        registerDismissEntry({
          id: 'confirm-high',
          getOnDismiss: () => vi.fn(),
          getOnConfirm: () => onConfirmHigh,
          getDisabled: () => false,
          priority: 20,
          lockScroll: false,
        }),
        registerDismissEntry({
          id: 'confirm-low',
          getOnDismiss: () => vi.fn(),
          getOnConfirm: () => onConfirmLow,
          getDisabled: () => false,
          priority: 0,
          lockScroll: false,
        }),
      )

      expect(handleGlobalConfirmKeyDown({ key: 'Enter' })).toBe(true)
      expect(onConfirmHigh).toHaveBeenCalledTimes(1)
      expect(onConfirmLow).not.toHaveBeenCalled()
    })

    it('does not run a disabled entry confirm but still consumes the key', () => {
      const onConfirm = vi.fn()
      const unreg = registerDismissEntry({
        id: 'confirm-disabled',
        getOnDismiss: () => vi.fn(),
        getOnConfirm: () => onConfirm,
        getDisabled: () => true,
        priority: 0,
        lockScroll: false,
      })
      cleanups.push(unreg)

      expect(handleGlobalConfirmKeyDown({ key: 'Enter' })).toBe(true)
      expect(onConfirm).not.toHaveBeenCalled()
    })

    it('does not treat Enter as confirm while an input holds focus', () => {
      const onConfirm = vi.fn()
      cleanups.push(
        registerDismissEntry({
          id: 'confirm-behind-input',
          getOnDismiss: () => vi.fn(),
          getOnConfirm: () => onConfirm,
          getDisabled: () => false,
          priority: 0,
          lockScroll: false,
        }),
      )
      vi.stubGlobal('document', { activeElement: { tagName: 'INPUT', isContentEditable: false } })

      // 输入态下 Enter 属于用户输入行为，不得升级为确认
      expect(handleGlobalConfirmKeyDown({ key: 'Enter' })).toBe(false)
      expect(onConfirm).not.toHaveBeenCalled()
    })

    it('ignores non-Enter keys', () => {
      const onConfirm = vi.fn()
      cleanups.push(
        registerDismissEntry({
          id: 'confirm-stack',
          getOnDismiss: () => vi.fn(),
          getOnConfirm: () => onConfirm,
          getDisabled: () => false,
          priority: 0,
          lockScroll: false,
        }),
      )

      expect(handleGlobalConfirmKeyDown({ key: 'Escape' })).toBe(false)
      expect(onConfirm).not.toHaveBeenCalled()
    })
  })
})
