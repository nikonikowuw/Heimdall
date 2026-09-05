import { describe, expect, it } from 'vitest'
import { parseWebCodecsFrame, WEBCODECS_HEADER_LEN } from './webcodecs'

describe('WebCodecs Framing Parser', () => {
  it('should parse valid H.264 keyframe buffer correctly', () => {
    const payload = new Uint8Array([0x00, 0x00, 0x00, 0x01, 0x67, 0x42])
    const buffer = new ArrayBuffer(WEBCODECS_HEADER_LEN + payload.length)
    const view = new DataView(buffer)

    view.setUint8(0, 0x01) // Version
    view.setUint8(1, 0x01) // H264
    view.setUint8(2, 0x01) // Keyframe
    view.setUint8(3, 0x00) // Flags
    view.setBigInt64(4, 1741100050123n, false) // PTS

    const raw = new Uint8Array(buffer)
    raw.set(payload, WEBCODECS_HEADER_LEN)

    const parsed = parseWebCodecsFrame(buffer)
    expect(parsed).not.toBeNull()
    expect(parsed?.version).toBe(1)
    expect(parsed?.codec).toBe('h264')
    expect(parsed?.isKeyframe).toBe(true)
    expect(parsed?.ptsMs).toBe(1741100050123)
    expect(parsed?.payload.length).toBe(payload.length)
    expect(parsed?.payload[4]).toBe(0x67)
  })

  it('should parse valid H.265 delta frame buffer correctly', () => {
    const payload = new Uint8Array([0x00, 0x00, 0x00, 0x01, 0x01])
    const buffer = new ArrayBuffer(WEBCODECS_HEADER_LEN + payload.length)
    const view = new DataView(buffer)

    view.setUint8(0, 0x01) // Version
    view.setUint8(1, 0x02) // H265
    view.setUint8(2, 0x00) // Delta
    view.setUint8(3, 0x00) // Flags
    view.setBigInt64(4, 1000n, false) // PTS

    const raw = new Uint8Array(buffer)
    raw.set(payload, WEBCODECS_HEADER_LEN)

    const parsed = parseWebCodecsFrame(buffer)
    expect(parsed).not.toBeNull()
    expect(parsed?.codec).toBe('h265')
    expect(parsed?.isKeyframe).toBe(false)
    expect(parsed?.ptsMs).toBe(1000)
  })

  it('should return null for truncated or invalid version buffer', () => {
    // 长度不足 12 字节
    const shortBuffer = new ArrayBuffer(8)
    expect(parseWebCodecsFrame(shortBuffer)).toBeNull()

    // 错误的版本号
    const badVerBuffer = new ArrayBuffer(16)
    new DataView(badVerBuffer).setUint8(0, 0x02)
    expect(parseWebCodecsFrame(badVerBuffer)).toBeNull()
  })
})
