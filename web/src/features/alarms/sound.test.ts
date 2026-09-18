import { describe, it, expect, beforeEach, vi } from 'vitest'
import { isAlarmSoundEnabled, setAlarmSoundEnabled, playAlarmAlertSound } from './sound'

describe('sound alert module', () => {
  let mockStorage: Record<string, string> = {}

  beforeEach(() => {
    mockStorage = {}
    const fakeLocalStorage = {
      getItem: vi.fn((key: string) => mockStorage[key] ?? null),
      setItem: vi.fn((key: string, val: string) => {
        mockStorage[key] = val
      }),
      clear: vi.fn(() => {
        mockStorage = {}
      }),
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(globalThis as any).window = {
      localStorage: fakeLocalStorage,
    }
  })

  it('defaults to true when storage is empty', () => {
    expect(isAlarmSoundEnabled()).toBe(true)
  })

  it('persists enabled state correctly', () => {
    setAlarmSoundEnabled(false)
    expect(isAlarmSoundEnabled()).toBe(false)
    setAlarmSoundEnabled(true)
    expect(isAlarmSoundEnabled()).toBe(true)
  })

  it('handles playAlarmAlertSound safely in mock environment', () => {
    const mockGain = {
      connect: vi.fn(),
      gain: {
        setValueAtTime: vi.fn(),
        exponentialRampToValueAtTime: vi.fn(),
      },
    }
    const mockOsc = {
      connect: vi.fn(),
      type: 'sine',
      frequency: {
        setValueAtTime: vi.fn(),
      },
      start: vi.fn(),
      stop: vi.fn(),
    }
    const mockContext = {
      currentTime: 0,
      state: 'running',
      destination: {},
      createGain: vi.fn(() => mockGain),
      createOscillator: vi.fn(() => mockOsc),
      resume: vi.fn().mockResolvedValue(undefined),
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(globalThis as any).window.AudioContext = vi.fn(() => mockContext)
    setAlarmSoundEnabled(true)
    expect(() => playAlarmAlertSound()).not.toThrow()
    expect(mockContext.createOscillator).toHaveBeenCalled()
  })

  it('suppresses sound when disabled', () => {
    const mockContext = {
      createGain: vi.fn(),
      createOscillator: vi.fn(),
    }
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    ;(globalThis as any).window.AudioContext = vi.fn(() => mockContext)
    setAlarmSoundEnabled(false)
    playAlarmAlertSound()
    expect(mockContext.createOscillator).not.toHaveBeenCalled()
  })
})
