import { afterEach, describe, it, expect, vi } from 'vitest'
import {
  calculateDownsampleDimensions,
  checkPersonnelPhotos,
  formatMiB,
  isWithinMultipartLimit,
  preparePersonnelPhotos,
  MAX_PERSONNEL_IMAGE_EDGE,
  MAX_PERSONNEL_MULTIPART_BYTES,
  PERSONNEL_MULTIPART_OVERHEAD_RESERVE_BYTES,
  MAX_PERSONNEL_PHOTO_BYTES,
  MAX_PERSONNEL_PHOTOS_PER_PERSON,
} from './personnelUpload'

/** 造一个只关心体积的伪 File */
function fileOf(name: string, bytes: number): File {
  return { name, size: bytes } as File
}

describe('checkPersonnelPhotos', () => {
  it('accepts photos within both the count and size limits', () => {
    const files = [
      fileOf('a.jpg', 1024),
      fileOf('b.jpg', 2 * 1024 * 1024),
      fileOf('c.jpg', MAX_PERSONNEL_PHOTO_BYTES),
    ]
    const result = checkPersonnelPhotos(files, MAX_PERSONNEL_PHOTOS_PER_PERSON)

    expect(result.ok.map((f) => f.name)).toEqual(['a.jpg', 'b.jpg', 'c.jpg'])
    expect(result.oversize).toEqual([])
    expect(result.overflow).toEqual([])
  })

  it('separates oversize photos instead of failing the whole batch', () => {
    const files = [
      fileOf('small.jpg', 1024),
      fileOf('huge.jpg', MAX_PERSONNEL_PHOTO_BYTES + 1),
      fileOf('ok.jpg', 2048),
    ]
    const result = checkPersonnelPhotos(files, MAX_PERSONNEL_PHOTOS_PER_PERSON)

    expect(result.ok.map((f) => f.name)).toEqual(['small.jpg', 'ok.jpg'])
    expect(result.oversize.map((f) => f.name)).toEqual(['huge.jpg'])
  })

  it('treats exactly-at-limit as acceptable, matching the backend comparison', () => {
    const result = checkPersonnelPhotos(
      [fileOf('edge.jpg', MAX_PERSONNEL_PHOTO_BYTES)],
      MAX_PERSONNEL_PHOTOS_PER_PERSON,
    )
    expect(result.ok).toHaveLength(1)
    expect(result.oversize).toEqual([])
  })

  it('truncates beyond the remaining slots and reports the extras', () => {
    const files = [1, 2, 3, 4].map((n) => fileOf(`${n}.jpg`, 1024))
    const result = checkPersonnelPhotos(files, 2)

    expect(result.ok.map((f) => f.name)).toEqual(['1.jpg', '2.jpg'])
    expect(result.overflow.map((f) => f.name)).toEqual(['3.jpg', '4.jpg'])
  })

  it('reports the count overflow before the size overflow', () => {
    // 第 3 张超出剩余槽位，同时第 1 张体积超限：
    // 用户该看到「张数超限」，而不是被体积错误盖住真正的原因
    const files = [
      fileOf('huge.jpg', MAX_PERSONNEL_PHOTO_BYTES + 1),
      fileOf('b.jpg', 1024),
      fileOf('c.jpg', 1024),
    ]
    const result = checkPersonnelPhotos(files, 2)

    expect(result.ok.map((f) => f.name)).toEqual(['b.jpg'])
    expect(result.oversize.map((f) => f.name)).toEqual(['huge.jpg'])
    expect(result.overflow.map((f) => f.name)).toEqual(['c.jpg'])
  })

  it('returns nothing acceptable when no slots remain', () => {
    const result = checkPersonnelPhotos([fileOf('a.jpg', 1024)], 0)

    expect(result.ok).toEqual([])
    expect(result.overflow.map((f) => f.name)).toEqual(['a.jpg'])
  })

  it('treats a negative slot count as zero rather than accepting everything', () => {
    const result = checkPersonnelPhotos([fileOf('a.jpg', 1024)], -3)

    expect(result.ok).toEqual([])
    expect(result.overflow).toHaveLength(1)
  })
})

describe('calculateDownsampleDimensions', () => {
  it('scales dimensions proportionally and clamps each edge to one pixel', () => {
    expect(calculateDownsampleDimensions(8192, 4096, 0.5)).toEqual({
      width: 4096,
      height: 2048,
    })
    expect(calculateDownsampleDimensions(0, 0, 0.5)).toEqual({ width: 1, height: 1 })
  })
})

describe('preparePersonnelPhotos', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('downsamples large dimensions with Canvas and releases the decoded bitmap', async () => {
    const drawImage = vi.fn()
    const clearRect = vi.fn()
    const fillRect = vi.fn()
    const bitmap = { width: 8192, height: 4096, close: vi.fn() }
    const context = { drawImage, clearRect, fillRect, fillStyle: '' }
    const canvas = {
      width: 0,
      height: 0,
      getContext: vi.fn(() => context),
      toBlob: (callback: BlobCallback) => callback(new Blob(['prepared'], { type: 'image/jpeg' })),
    }
    vi.stubGlobal(
      'createImageBitmap',
      vi.fn(async () => bitmap),
    )
    vi.stubGlobal('document', { createElement: vi.fn(() => canvas) })

    const source = new File(['source'], 'portrait.jpg', {
      type: 'image/jpeg',
      lastModified: 123,
    })
    const result = await preparePersonnelPhotos([source])

    expect(result.failed).toEqual([])
    expect(result.files).toHaveLength(1)
    expect(result.files[0].name).toBe('portrait.jpg')
    expect(result.files[0].type).toBe('image/jpeg')
    expect(canvas.width).toBe(MAX_PERSONNEL_IMAGE_EDGE)
    expect(canvas.height).toBe(MAX_PERSONNEL_IMAGE_EDGE / 2)
    expect(drawImage).toHaveBeenCalledWith(bitmap, 0, 0, canvas.width, canvas.height)
    expect(clearRect).toHaveBeenCalledOnce()
    expect(fillRect).toHaveBeenCalledOnce()
    expect(bitmap.close).toHaveBeenCalledOnce()
  })

  it('reduces an oversized file even when its dimensions are already small', async () => {
    const bitmap = { width: 1000, height: 500, close: vi.fn() }
    const canvas = {
      width: 0,
      height: 0,
      getContext: vi.fn(() => ({
        drawImage: vi.fn(),
        clearRect: vi.fn(),
        fillRect: vi.fn(),
        fillStyle: '',
      })),
      toBlob: (callback: BlobCallback) => callback(new Blob(['prepared'], { type: 'image/jpeg' })),
    }
    vi.stubGlobal(
      'createImageBitmap',
      vi.fn(async () => bitmap),
    )
    vi.stubGlobal('document', { createElement: vi.fn(() => canvas) })
    const source = fileOf('oversized.jpg', MAX_PERSONNEL_PHOTO_BYTES + 1) as File

    const result = await preparePersonnelPhotos([source])

    expect(result.failed).toEqual([])
    expect(result.files).toHaveLength(1)
    expect(canvas.width).toBeLessThan(bitmap.width)
    expect(canvas.height).toBeLessThan(bitmap.height)
    expect(bitmap.close).toHaveBeenCalledOnce()
  })

  it('reports a file when browser image decoding fails', async () => {
    vi.stubGlobal('createImageBitmap', vi.fn().mockRejectedValue(new Error('invalid image')))
    const source = new File(['source'], 'invalid.jpg', { type: 'image/jpeg' })

    const result = await preparePersonnelPhotos([source])

    expect(result.files).toEqual([])
    expect(result.failed).toEqual([source])
  })
})

describe('isWithinMultipartLimit', () => {
  it('accepts a photo payload that leaves the multipart overhead reserve', () => {
    const photosTotal = MAX_PERSONNEL_MULTIPART_BYTES - PERSONNEL_MULTIPART_OVERHEAD_RESERVE_BYTES
    expect(isWithinMultipartLimit([fileOf('a.jpg', photosTotal)])).toBe(true)
  })

  it('rejects a payload that consumes the multipart overhead reserve', () => {
    const photosTotal =
      MAX_PERSONNEL_MULTIPART_BYTES - PERSONNEL_MULTIPART_OVERHEAD_RESERVE_BYTES + 1
    expect(isWithinMultipartLimit([fileOf('a.jpg', photosTotal)])).toBe(false)
  })

  it('sums across files', () => {
    const perFile = (MAX_PERSONNEL_MULTIPART_BYTES - PERSONNEL_MULTIPART_OVERHEAD_RESERVE_BYTES) / 4
    expect(isWithinMultipartLimit([1, 2, 3, 4].map((n) => fileOf(`${n}.jpg`, perFile)))).toBe(true)
    expect(isWithinMultipartLimit([1, 2, 3, 4, 5].map((n) => fileOf(`${n}.jpg`, perFile)))).toBe(
      false,
    )
  })
})

describe('the limits stay in sync with the backend contract', () => {
  // 这三条断言锁定的是「与 Rust 侧同源」这件事本身。
  // 后端改动而前端漏改时，这里会先红，而不是等现场上传才暴露。
  it('matches MAX_PERSONNEL_PHOTO_BYTES = 12 MiB', () => {
    expect(MAX_PERSONNEL_PHOTO_BYTES).toBe(12 * 1024 * 1024)
    expect(formatMiB(MAX_PERSONNEL_PHOTO_BYTES)).toBe(12)
  })

  it('matches MAX_PERSONNEL_MULTIPART_BYTES = 64 MiB', () => {
    expect(MAX_PERSONNEL_MULTIPART_BYTES).toBe(64 * 1024 * 1024)
    expect(formatMiB(MAX_PERSONNEL_MULTIPART_BYTES)).toBe(64)
  })

  it('matches MAX_PERSONNEL_PHOTOS_PER_PERSON = 5', () => {
    expect(MAX_PERSONNEL_PHOTOS_PER_PERSON).toBe(5)
  })
})
