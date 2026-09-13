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
})
