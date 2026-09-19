import { beforeEach, describe, expect, it } from 'vitest'
import { MAX_TOASTS, toast, useToastStore } from './toast'

describe('toast store', () => {
  beforeEach(() => {
    useToastStore.getState().clearToasts()
  })

  it('adds and dismisses a toast item correctly', () => {
    const id = toast.success('Operation succeeded', { title: 'Success Title' })
    const { toasts } = useToastStore.getState()

    expect(toasts).toHaveLength(1)
    expect(toasts[0].id).toBe(id)
    expect(toasts[0].type).toBe('success')
    expect(toasts[0].title).toBe('Success Title')
    expect(toasts[0].message).toBe('Operation succeeded')

    toast.dismiss(id)
    expect(useToastStore.getState().toasts).toHaveLength(0)
  })

  it('supports different semantic types (error, warning, info)', () => {
    toast.error('Fatal error occurred')
    toast.warning('Attention required')
    toast.info('System online')

    const items = useToastStore.getState().toasts
    expect(items).toHaveLength(3)
    expect(items[0].type).toBe('info')
    expect(items[1].type).toBe('warning')
    expect(items[2].type).toBe('error')
  })

  it('enforces maximum toast bound to prevent unbounded growth', () => {
    for (let i = 0; i < MAX_TOASTS + 3; i++) {
      toast.info(`Message ${i}`)
    }

    const { toasts } = useToastStore.getState()
    expect(toasts).toHaveLength(MAX_TOASTS)
    // 最新的排在最上面
    expect(toasts[0].message).toBe(`Message ${MAX_TOASTS + 2}`)
  })

  it('supports object argument signature with category and action', () => {
    let actionTriggered = false
    const id = toast.show({
      type: 'warning',
      category: 'DEVICE_PROBE',
      title: 'Probe Warning',
      message: 'Packet loss detected',
      duration: 5000,
      action: {
        label: 'Retry',
        onClick: () => {
          actionTriggered = true
        },
      },
    })

    const item = useToastStore.getState().toasts.find((t) => t.id === id)
    expect(item).toBeDefined()
    expect(item?.category).toBe('DEVICE_PROBE')
    expect(item?.duration).toBe(5000)
    expect(item?.action?.label).toBe('Retry')

    item?.action?.onClick({} as React.MouseEvent<HTMLButtonElement>)
    expect(actionTriggered).toBe(true)
  })

  it('allows clearing all toasts', () => {
    toast.info('1')
    toast.info('2')
    expect(useToastStore.getState().toasts).toHaveLength(2)

    toast.clear()
    expect(useToastStore.getState().toasts).toHaveLength(0)
  })

  it('enforces semantic method type even if input object specifies another type', () => {
    const id = toast.success({
      message: 'Success occurred',
      type: 'error',
    } as unknown as { message: string })

    const item = useToastStore.getState().toasts.find((t) => t.id === id)
    expect(item?.type).toBe('success')
  })
})
