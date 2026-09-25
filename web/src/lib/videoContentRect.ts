export type VideoFitMode = 'contain' | 'cover' | 'fill'

export interface VideoContentRect {
  x: number
  y: number
  width: number
  height: number
}

export function getVideoContentRect(
  viewWidth: number,
  viewHeight: number,
  sourceWidth: number,
  sourceHeight: number,
  fitMode: VideoFitMode,
): VideoContentRect | null {
  if (
    !Number.isFinite(viewWidth) ||
    !Number.isFinite(viewHeight) ||
    !Number.isFinite(sourceWidth) ||
    !Number.isFinite(sourceHeight) ||
    viewWidth <= 0 ||
    viewHeight <= 0 ||
    sourceWidth <= 0 ||
    sourceHeight <= 0
  ) {
    return null
  }

  if (fitMode === 'fill') {
    return { x: 0, y: 0, width: viewWidth, height: viewHeight }
  }

  const widthScale = viewWidth / sourceWidth
  const heightScale = viewHeight / sourceHeight
  const scale =
    fitMode === 'contain' ? Math.min(widthScale, heightScale) : Math.max(widthScale, heightScale)
  const width = sourceWidth * scale
  const height = sourceHeight * scale

  return {
    x: (viewWidth - width) / 2,
    y: (viewHeight - height) / 2,
    width,
    height,
  }
}
