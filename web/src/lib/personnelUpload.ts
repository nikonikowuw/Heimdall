/**
 * 人员照片上传的前置校验。
 *
 * 与后端限制同源（`crates/api/src/personnel_service.rs`）：
 * - 单张照片 12 MiB（`MAX_PERSONNEL_PHOTO_BYTES`）
 * - 单人照片数 5 张（`MAX_PERSONNEL_PHOTOS_PER_PERSON`）
 * - 单次表单 64 MiB（`MAX_PERSONNEL_MULTIPART_BYTES`）
 *
 * 为什么必须在客户端先拦：超限时后端返回 400 且**不落任何数据**，
 * 但用户已经等了一次完整上传；更糟的是若前端不校验而表单整体超 64 MiB，
 * 拿到的是「表单总大小超限」而不是「哪一张图太大」，归因成本高。
 */

/** 单张照片上限（字节），与后端 `MAX_PERSONNEL_PHOTO_BYTES` 一致 */
export const MAX_PERSONNEL_PHOTO_BYTES = 12 * 1024 * 1024

/** 单人照片数上限，与后端 `MAX_PERSONNEL_PHOTOS_PER_PERSON` 一致 */
export const MAX_PERSONNEL_PHOTOS_PER_PERSON = 5

/** 单次 multipart 表单上限（字节），与后端 `MAX_PERSONNEL_MULTIPART_BYTES` 一致 */
export const MAX_PERSONNEL_MULTIPART_BYTES = 64 * 1024 * 1024

/** 保留 multipart 边界与非文件字段空间，避免 64 MiB 文件恰好卡在 HTTP body 上限。 */
export const PERSONNEL_MULTIPART_OVERHEAD_RESERVE_BYTES = 64 * 1024

/** 超过该边长或文件上限时，前端使用 Canvas 缩小并重编码。 */
export const MAX_PERSONNEL_IMAGE_EDGE = 4096

/** 校验结果：`ok` 为可接受的照片，超限项按原因分类以便逐条提示 */
export interface PhotoSelectionCheck {
  /** 按原始顺序保留且未超出张数上限的照片 */
  selected: File[]
  ok: File[]
  /** 单张超限的文件 */
  oversize: File[]
  /** 因超出单人张数上限而被截断的文件 */
  overflow: File[]
}

/** 相对后端的文件大小格式，用于提示文案（12 → "12 MiB"） */
export function formatMiB(bytes: number): number {
  return Math.round(bytes / (1024 * 1024))
}

/**
 * 按后端口径筛选一批待上传照片。
 *
 * 顺序敏感：先按张数截断，再判单张体积。这样「选了 8 张其中第 7 张超大」
 * 只会提示张数超限，而不是用一个用户无法处理的体积错误盖住真正的原因。
 */
export function checkPersonnelPhotos(
  files: readonly File[],
  availableSlots: number,
): PhotoSelectionCheck {
  const slots = Math.max(0, Math.trunc(availableSlots))
  const selected: File[] = []
  const overflow: File[] = []

  for (const file of files) {
    if (selected.length >= slots) {
      overflow.push(file)
      continue
    }
    selected.push(file)
  }

  const ok: File[] = []
  const oversize: File[] = []
  for (const file of selected) {
    if (file.size > MAX_PERSONNEL_PHOTO_BYTES) {
      oversize.push(file)
    } else {
      ok.push(file)
    }
  }

  return { selected, ok, oversize, overflow }
}

/** 以比例缩放照片尺寸，并确保输出边长至少为一个像素。 */
export function calculateDownsampleDimensions(
  width: number,
  height: number,
  scale: number,
): { width: number; height: number } {
  return {
    width: Math.max(1, Math.floor(width * scale)),
    height: Math.max(1, Math.floor(height * scale)),
  }
}

async function canvasToBlob(
  canvas: HTMLCanvasElement,
  type: string,
  quality: number,
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new Error('Canvas returned no image data'))),
      type,
      quality,
    )
  })
}

async function preparePersonnelPhoto(file: File): Promise<File> {
  if (typeof createImageBitmap !== 'function') {
    if (file.size <= MAX_PERSONNEL_PHOTO_BYTES) return file
    throw new Error('Image decoding is unavailable')
  }

  const bitmap = await createImageBitmap(file)
  try {
    const maxEdge = Math.max(bitmap.width, bitmap.height)
    if (maxEdge <= MAX_PERSONNEL_IMAGE_EDGE && file.size <= MAX_PERSONNEL_PHOTO_BYTES) {
      return file
    }

    const byteScale =
      file.size > MAX_PERSONNEL_PHOTO_BYTES
        ? Math.sqrt(MAX_PERSONNEL_PHOTO_BYTES / file.size) * 0.95
        : 1
    let scale = Math.min(1, MAX_PERSONNEL_IMAGE_EDGE / maxEdge, byteScale)
    const mimeType = ['image/jpeg', 'image/png', 'image/webp'].includes(file.type)
      ? file.type
      : 'image/jpeg'
    const quality = mimeType === 'image/png' ? undefined : 0.9
    const canvas = document.createElement('canvas')
    const context = canvas.getContext('2d')
    if (!context) throw new Error('Canvas 2D context is unavailable')

    for (let attempt = 0; attempt < 6; attempt += 1) {
      const dimensions = calculateDownsampleDimensions(bitmap.width, bitmap.height, scale)
      canvas.width = dimensions.width
      canvas.height = dimensions.height
      context.clearRect(0, 0, canvas.width, canvas.height)
      if (mimeType === 'image/jpeg') {
        context.fillStyle = '#fff'
        context.fillRect(0, 0, canvas.width, canvas.height)
      }
      context.drawImage(bitmap, 0, 0, canvas.width, canvas.height)
      const blob = await canvasToBlob(canvas, mimeType, quality ?? 1)
      if (blob.size <= MAX_PERSONNEL_PHOTO_BYTES) {
        const outputType = blob.type || mimeType
        const extension =
          outputType === 'image/png' ? 'png' : outputType === 'image/webp' ? 'webp' : 'jpg'
        const stem = file.name.replace(/\.[^.]+$/, '') || 'photo'
        return new File([blob], `${stem}.${extension}`, {
          type: outputType,
          lastModified: file.lastModified,
        })
      }
      scale *= 0.75
    }

    throw new Error('Photo cannot be reduced below the upload limit')
  } finally {
    bitmap.close()
  }
}

/** 串行解码和缩放，避免同时创建多张高分辨率 Canvas 占用峰值内存。 */
export async function preparePersonnelPhotos(
  files: readonly File[],
): Promise<{ files: File[]; failed: File[] }> {
  const prepared: File[] = []
  const failed: File[] = []

  for (const file of files) {
    try {
      prepared.push(await preparePersonnelPhoto(file))
    } catch {
      failed.push(file)
    }
  }

  return { files: prepared, failed }
}

/** 已选照片的合计体积是否仍在单次表单上限内 */
export function isWithinMultipartLimit(files: readonly File[]): boolean {
  const photoBytes = files.reduce((total, file) => total + file.size, 0)
  return photoBytes + PERSONNEL_MULTIPART_OVERHEAD_RESERVE_BYTES <= MAX_PERSONNEL_MULTIPART_BYTES
}
