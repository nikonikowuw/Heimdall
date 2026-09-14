import { describe, expect, it } from 'vitest'
import { deriveDownloadFilename } from '../utils'

describe('ImagePreviewModal deriveDownloadFilename', () => {
  it('uses customFilename when provided', () => {
    const filename = deriveDownloadFilename(
      '/api/v1/evidence/images/some_opaque_hash.jpg',
      'rec_12345_crop.jpg',
      '现场抓拍',
    )
    expect(filename).toBe('rec_12345_crop.jpg')
  })

  it('extracts filename from image URL pathname if customFilename is not specified', () => {
    const filename = deriveDownloadFilename('/api/v1/evidence/images/alarm_event_9988.jpg')
    expect(filename).toBe('alarm_event_9988.jpg')
  })

  it('extracts raw image filename ignoring token query parameters and directory path', () => {
    const filename = deriveDownloadFilename(
      '/api/v1/evidence/image/galleries/sub_alice/original_face_1.jpg?token=abcxyz123',
    )
    expect(filename).toBe('original_face_1.jpg')
  })

  it('sanitizes title when url does not end with image extension and no custom filename', () => {
    const filename = deriveDownloadFilename(
      '/api/v1/evidence/raw_stream?id=12',
      undefined,
      '通道 1: 告警特写',
    )
    expect(filename).toBe('通道_1_告警特写.jpg')
  })

  it('falls back to timestamped filename when no info is given', () => {
    const filename = deriveDownloadFilename('/api/v1/stream')
    expect(filename).toMatch(/^image_\d+\.jpg$/)
  })
})
