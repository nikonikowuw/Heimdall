import { afterEach, describe, expect, it, vi } from 'vitest'
import { cn, copyToClipboard } from './utils'

describe('cn utility function', () => {
  it('should merge class names correctly', () => {
    const result = cn('bg-red-500', 'text-white', { 'p-4': true, 'p-2': false })
    expect(result).toBe('bg-red-500 text-white p-4')
  })

  it('should resolve tailwind conflict classes', () => {
    const result = cn('px-2', 'px-4')
    expect(result).toBe('px-4')
  })
})

describe('copyToClipboard', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('should return false if text is empty', async () => {
    expect(await copyToClipboard('')).toBe(false)
  })

  it('should use modern navigator.clipboard.writeText when in secure context', async () => {
    const writeTextMock = vi.fn().mockResolvedValue(undefined)

    vi.stubGlobal('window', { isSecureContext: true })
    vi.stubGlobal('navigator', { clipboard: { writeText: writeTextMock } })

    const success = await copyToClipboard('rtsp://admin:123456@192.168.1.100:554/live')
    expect(success).toBe(true)
    expect(writeTextMock).toHaveBeenCalledWith('rtsp://admin:123456@192.168.1.100:554/live')
  })

  it('should fallback to execCommand when navigator.clipboard is undefined (e.g. HTTP LAN)', async () => {
    const execCommandMock = vi.fn().mockReturnValue(true)
    const appendChildMock = vi.fn()
    const removeChildMock = vi.fn()

    const mockTextArea = {
      value: '',
      style: {} as Record<string, string>,
      setAttribute: vi.fn(),
      focus: vi.fn(),
      select: vi.fn(),
      setSelectionRange: vi.fn(),
    }

    // 模拟非安全上下文（如局域网 HTTP 部署）
    vi.stubGlobal('window', { isSecureContext: false })
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('document', {
      createElement: vi.fn().mockReturnValue(mockTextArea),
      body: {
        appendChild: appendChildMock,
        removeChild: removeChildMock,
      },
      execCommand: execCommandMock,
    })

    const testUrl = 'rtsp://192.168.1.10:554/stream1'
    const success = await copyToClipboard(testUrl)

    expect(success).toBe(true)
    expect(mockTextArea.value).toBe(testUrl)
    expect(appendChildMock).toHaveBeenCalledWith(mockTextArea)
    expect(execCommandMock).toHaveBeenCalledWith('copy')
    expect(removeChildMock).toHaveBeenCalledWith(mockTextArea)
  })

  it('should fallback to execCommand when navigator.clipboard.writeText rejects', async () => {
    const writeTextMock = vi.fn().mockRejectedValue(new Error('Permission denied'))
    const execCommandMock = vi.fn().mockReturnValue(true)
    const appendChildMock = vi.fn()
    const removeChildMock = vi.fn()

    const mockTextArea = {
      value: '',
      style: {} as Record<string, string>,
      setAttribute: vi.fn(),
      focus: vi.fn(),
      select: vi.fn(),
      setSelectionRange: vi.fn(),
    }

    vi.stubGlobal('window', { isSecureContext: true })
    vi.stubGlobal('navigator', { clipboard: { writeText: writeTextMock } })
    vi.stubGlobal('document', {
      createElement: vi.fn().mockReturnValue(mockTextArea),
      body: {
        appendChild: appendChildMock,
        removeChild: removeChildMock,
      },
      execCommand: execCommandMock,
    })

    const testUrl = 'rtsp://192.168.1.10:554/stream1'
    const success = await copyToClipboard(testUrl)

    expect(writeTextMock).toHaveBeenCalledWith(testUrl)
    expect(success).toBe(true)
    expect(execCommandMock).toHaveBeenCalledWith('copy')
  })

  it('should return false if both modern API and execCommand fail or throw', async () => {
    vi.stubGlobal('window', { isSecureContext: false })
    vi.stubGlobal('navigator', {})
    vi.stubGlobal('document', {
      createElement: vi.fn().mockImplementation(() => {
        throw new Error('DOM manipulation not allowed')
      }),
    })

    const success = await copyToClipboard('rtsp://192.168.1.10:554/stream1')
    expect(success).toBe(false)
  })
})
