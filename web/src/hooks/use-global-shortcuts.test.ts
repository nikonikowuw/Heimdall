import { describe, expect, it, vi } from 'vitest'
import { handleGlobalShortcutEvent } from './use-global-shortcuts'

describe('handleGlobalShortcutEvent', () => {
  const createOptions = () => ({
    onSelectTab: vi.fn(),
    onToggleTheme: vi.fn(),
    onOpenShortcutsHelp: vi.fn(),
    isModalOpen: () => false,
    focusSearchInput: vi.fn(() => true),
  })

  it('switches tabs with numbers 1-8 in idle mode', () => {
    const opts = createOptions()
    const consumed = handleGlobalShortcutEvent(
      {
        key: '1',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(consumed).toBe(true)
    expect(opts.onSelectTab).toHaveBeenCalledWith('live')

    handleGlobalShortcutEvent(
      {
        key: '6',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(opts.onSelectTab).toHaveBeenCalledWith('alarms')
  })

  it('opens shortcuts help with "?"', () => {
    const opts = createOptions()
    const consumed = handleGlobalShortcutEvent(
      {
        key: '?',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: true,
      },
      opts,
    )
    expect(consumed).toBe(true)
    expect(opts.onOpenShortcutsHelp).toHaveBeenCalledTimes(1)
  })

  it('toggles theme with "t"', () => {
    const opts = createOptions()
    const consumed = handleGlobalShortcutEvent(
      {
        key: 't',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(consumed).toBe(true)
    expect(opts.onToggleTheme).toHaveBeenCalledTimes(1)
  })

  it('does not trigger shortcuts when modal is open', () => {
    const opts = {
      ...createOptions(),
      isModalOpen: () => true,
    }
    const consumed = handleGlobalShortcutEvent(
      {
        key: '1',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(consumed).toBe(false)
    expect(opts.onSelectTab).not.toHaveBeenCalled()
  })

  it('does not intercept browser Cmd/Ctrl combinations', () => {
    const opts = createOptions()
    const consumed = handleGlobalShortcutEvent(
      {
        key: '1',
        altKey: false,
        ctrlKey: true,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(consumed).toBe(false)
    expect(opts.onSelectTab).not.toHaveBeenCalled()
  })

  /*
   * '/' 聚焦搜索框。
   *
   * 任务列表检索契约要求该快捷键在任务页聚焦筛选框，且模态打开时不得抢焦点。
   * 这三条直接钉住验收标准，替代原先仅有的手工核对。
   */

  it('focuses the search input with "/" and consumes the event', () => {
    const opts = createOptions()
    const consumed = handleGlobalShortcutEvent(
      {
        key: '/',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(consumed).toBe(true)
    expect(opts.focusSearchInput).toHaveBeenCalledTimes(1)
  })

  it('leaves "/" unconsumed when the page has no search input', () => {
    const opts = {
      ...createOptions(),
      focusSearchInput: vi.fn(() => false),
    }
    const consumed = handleGlobalShortcutEvent(
      {
        key: '/',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    // 不消费事件：无搜索框的页面不应把 '/' 吞掉（用户可能正想输入该字符）
    expect(consumed).toBe(false)
  })

  it('does not steal focus with "/" while a modal is open', () => {
    const opts = {
      ...createOptions(),
      isModalOpen: () => true,
    }
    const consumed = handleGlobalShortcutEvent(
      {
        key: '/',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: false,
      },
      opts,
    )
    expect(consumed).toBe(false)
    expect(opts.focusSearchInput).not.toHaveBeenCalled()
  })

  it('keeps Shift+/ bound to the help panel instead of the search box', () => {
    const opts = createOptions()
    const consumed = handleGlobalShortcutEvent(
      {
        key: '/',
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        shiftKey: true,
      },
      opts,
    )
    expect(consumed).toBe(true)
    expect(opts.onOpenShortcutsHelp).toHaveBeenCalledTimes(1)
    expect(opts.focusSearchInput).not.toHaveBeenCalled()
  })
})
