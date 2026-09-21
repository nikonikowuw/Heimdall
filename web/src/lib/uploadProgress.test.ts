import { describe, expect, it } from 'vitest'
import { isAlgorithmUploadProgress } from './uploadProgress'

describe('isAlgorithmUploadProgress', () => {
  it('accepts a valid device progress event', () => {
    expect(
      isAlgorithmUploadProgress({
        uploadId: 'upload-1',
        step: 4,
        status: 'passed',
      }),
    ).toBe(true)
  })

  it('rejects malformed or out-of-range payloads', () => {
    expect(isAlgorithmUploadProgress(null)).toBe(false)
    expect(
      isAlgorithmUploadProgress({
        uploadId: 'upload-1',
        step: 0,
        status: 'running',
      }),
    ).toBe(false)
    expect(
      isAlgorithmUploadProgress({
        uploadId: 'upload-1',
        step: 7,
        status: 'passed',
      }),
    ).toBe(false)
    expect(
      isAlgorithmUploadProgress({
        uploadId: 'upload-1',
        step: 2,
        status: 'unknown',
      }),
    ).toBe(false)
  })
})
