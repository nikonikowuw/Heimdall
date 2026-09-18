/**
 * 基于 Web Audio API 的轻量级告警警示音合成器
 * 无需外挂 mp3 资源，无网络依赖，毫秒级即时发声
 */

const STORAGE_KEY = 'heimdall_alarm_sound_enabled'

/**
 * 读取本地告警提示音启用状态（默认开启）
 */
export function isAlarmSoundEnabled(): boolean {
  try {
    if (typeof window === 'undefined' || !window.localStorage) return true
    const val = window.localStorage.getItem(STORAGE_KEY)
    return val === null ? true : val === 'true'
  } catch {
    return true
  }
}

/**
 * 切换告警提示音启用状态
 */
export function setAlarmSoundEnabled(enabled: boolean): void {
  try {
    if (typeof window !== 'undefined' && window.localStorage) {
      window.localStorage.setItem(STORAGE_KEY, String(enabled))
    }
  } catch {
    // 忽略私密浏览或存储限制
  }
}

let sharedAudioCtx: AudioContext | null = null

function getAudioContext(): AudioContext | null {
  if (typeof window === 'undefined') return null
  const AudioCtxClass =
    window.AudioContext ||
    (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
  if (!AudioCtxClass) return null

  if (!sharedAudioCtx || sharedAudioCtx.state === 'closed') {
    sharedAudioCtx = new AudioCtxClass()
  }
  return sharedAudioCtx
}

/**
 * 播放安防警示双音节提示音 (880Hz -> 660Hz 双脉冲)
 */
export function playAlarmAlertSound(): void {
  if (!isAlarmSoundEnabled()) return

  try {
    const ctx = getAudioContext()
    if (!ctx) return

    // 处理浏览器挂起的 AudioContext
    if (ctx.state === 'suspended') {
      ctx.resume().catch(() => {})
    }

    const now = ctx.currentTime

    // 第一声脉冲: 高音 880Hz (A5)
    const osc1 = ctx.createOscillator()
    const gain1 = ctx.createGain()
    osc1.type = 'sine'
    osc1.frequency.setValueAtTime(880, now)
    gain1.gain.setValueAtTime(0.25, now)
    gain1.gain.exponentialRampToValueAtTime(0.001, now + 0.12)

    osc1.connect(gain1)
    gain1.connect(ctx.destination)
    osc1.start(now)
    osc1.stop(now + 0.12)

    // 第二声脉冲: 稍低音 660Hz (E5)，间隔 140ms
    const osc2 = ctx.createOscillator()
    const gain2 = ctx.createGain()
    osc2.type = 'sine'
    osc2.frequency.setValueAtTime(660, now + 0.14)
    gain2.gain.setValueAtTime(0.3, now + 0.14)
    gain2.gain.exponentialRampToValueAtTime(0.001, now + 0.28)

    osc2.connect(gain2)
    gain2.connect(ctx.destination)
    osc2.start(now + 0.14)
    osc2.stop(now + 0.28)
  } catch {
    // 降级防御，静默吞没未捕获的音频策略异常
  }
}
