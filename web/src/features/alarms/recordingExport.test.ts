import { readFileSync } from 'node:fs'
import { ALL_FORMATS, BufferSource, Input } from 'mediabunny'
import type { InputVideoTrack } from 'mediabunny'
import { describe, expect, it } from 'vitest'
import type { RecordingDetail } from '@/types'
import { buildExportFileName, remuxFmp4ToMp4 } from './recordingExport'

const fixture = readFileSync(new URL('./__fixtures__/fmp4-sample.mp4', import.meta.url))
const fixtureBuffer = fixture.buffer.slice(
  fixture.byteOffset,
  fixture.byteOffset + fixture.byteLength,
) as ArrayBuffer

/** 在原始字节里查找 box 类型标记，用于区分分片与普通 MP4 结构 */
function hasBox(buffer: ArrayBuffer, tag: string): boolean {
  const bytes = new Uint8Array(buffer)
  const needle = [...tag].map((char) => char.charCodeAt(0))
  return bytes.some((_, index) => needle.every((code, offset) => bytes[index + offset] === code))
}

async function readVideoTrack(buffer: ArrayBuffer): Promise<InputVideoTrack | null> {
  const input = new Input({ source: new BufferSource(buffer), formats: ALL_FORMATS })
  try {
    const tracks = await input.getTracks()
    for (const track of tracks) {
      if (track.isVideoTrack()) return track
    }
    return null
  } finally {
    input.dispose()
  }
}

function makeRecording(overrides: Partial<RecordingDetail> = {}): RecordingDetail {
  return {
    id: 1,
    recordingId: 'rec-0001',
    cameraId: 'cam-001',
    startTime: Date.UTC(2026, 9, 1, 2, 30, 15),
    endTime: Date.UTC(2026, 9, 1, 2, 30, 25),
    durationMs: 10_000,
    fileSize: 1024,
    codec: 'h264',
    status: 'completed',
    createdAt: Date.UTC(2026, 9, 1, 2, 30, 26),
    events: [],
    ...overrides,
  }
}

describe('remuxFmp4ToMp4', () => {
  it('把分片 MP4 重建为带完整样本表的非分片 MP4', async () => {
    // 输入为 moof/mdat 分片（与后端 fMP4 writer 输出同构：无 sidx）
    expect(hasBox(fixtureBuffer, 'moof')).toBe(true)

    const output = await remuxFmp4ToMp4(fixtureBuffer)

    expect(hasBox(output, 'moof')).toBe(false)
    expect(hasBox(output, 'mvex')).toBe(false)
    expect(hasBox(output, 'moov')).toBe(true)
    expect(hasBox(output, 'mdat')).toBe(true)
    // 样本表齐全才能被外部工具随机访问
    for (const table of ['stts', 'stsz', 'stco', 'stsc']) {
      expect(hasBox(output, table)).toBe(true)
    }
  })

  it('产物可被重新解析且保留视频轨参数', async () => {
    const output = await remuxFmp4ToMp4(fixtureBuffer)
    const track = await readVideoTrack(output)

    expect(track).not.toBeNull()
    if (!track) throw new Error('remuxed output has no video track')
    expect(track.codec).toBe('avc')
    expect(track.displayWidth).toBe(96)
    expect(track.displayHeight).toBe(96)
  })

  it('拒绝无法解析的输入而不是产出残缺文件', async () => {
    await expect(remuxFmp4ToMp4(new ArrayBuffer(64))).rejects.toThrow()
  })
})

describe('buildExportFileName', () => {
  it('包含通道 ID、本地时间戳与 mp4 后缀', () => {
    const name = buildExportFileName(makeRecording({ cameraId: 'cam-001' }))
    expect(name).toMatch(/^recording_cam-001_\d{8}_\d{6}\.mp4$/)
  })
})
