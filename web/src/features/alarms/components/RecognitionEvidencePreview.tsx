import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
  calculateFittedImageRect,
  getBBoxStyle,
  parseTargetBBoxes,
  type FittedImageRect,
} from '../utils'

export interface RecognitionEvidencePreviewProps {
  src?: string | null
  bboxJson?: string | null
  alt: string
  label?: string
  className: string
  title?: string
  noImageText: string
  faceLabel: string
  onPreview?: () => void
}

export function RecognitionEvidencePreview({
  src,
  bboxJson,
  alt,
  label,
  className,
  title,
  noImageText,
  faceLabel,
  onPreview,
}: RecognitionEvidencePreviewProps): React.ReactElement {
  const containerRef = useRef<HTMLDivElement>(null)
  const imageRef = useRef<HTMLImageElement>(null)
  const [imgRect, setImgRect] = useState<FittedImageRect | null>(null)
  const targetBBoxes = parseTargetBBoxes(bboxJson ?? undefined)

  const updateImageRect = useCallback((): void => {
    const container = containerRef.current
    const image = imageRef.current
    if (!container || !image || !image.naturalWidth || !image.naturalHeight) return
    setImgRect(
      calculateFittedImageRect(
        container.clientWidth,
        container.clientHeight,
        image.naturalWidth,
        image.naturalHeight,
      ),
    )
  }, [])

  useEffect(() => {
    const container = containerRef.current
    if (!container || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(updateImageRect)
    observer.observe(container)
    return () => observer.disconnect()
  }, [updateImageRect])

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>): void {
    if (!onPreview || (event.key !== 'Enter' && event.key !== ' ')) return
    event.preventDefault()
    onPreview()
  }

  const isInteractive = Boolean(src && onPreview)
  const commonClassName = `group/evidence relative flex items-center justify-center overflow-hidden rounded-xl border border-[var(--border)] bg-black shadow-xs ${className} ${isInteractive ? 'cursor-pointer hover:border-[var(--accent)] hover:shadow-md' : ''}`

  return (
    <div
      ref={containerRef}
      className={commonClassName}
      title={isInteractive ? title : undefined}
      role={isInteractive ? 'button' : undefined}
      tabIndex={isInteractive ? 0 : undefined}
      onClick={isInteractive ? onPreview : undefined}
      onKeyDown={isInteractive ? handleKeyDown : undefined}
    >
      {src ? (
        <>
          <img
            ref={imageRef}
            src={src}
            alt={alt}
            className="h-full w-full object-contain"
            onLoad={updateImageRect}
          />
          {imgRect && targetBBoxes?.body && (
            <div
              className="pointer-events-none absolute border-2 border-cyan-400 bg-cyan-400/10 shadow-[0_0_12px_rgba(6,182,212,0.5)]"
              style={getBBoxStyle(targetBBoxes.body, imgRect)}
            />
          )}
          {imgRect && targetBBoxes?.face && (
            <div
              className="pointer-events-none absolute border-2 border-dashed border-purple-400 bg-purple-500/15 shadow-[0_0_12px_rgba(168,85,247,0.5)]"
              style={getBBoxStyle(targetBBoxes.face.bbox, imgRect)}
            >
              <span className="absolute -top-4 left-0 rounded bg-purple-600 px-1.5 py-0.5 font-mono text-[8px] font-bold whitespace-nowrap text-white shadow-xs">
                {targetBBoxes.face.qualityScore !== undefined
                  ? `${faceLabel} ${(targetBBoxes.face.qualityScore * 100).toFixed(0)}%`
                  : faceLabel}
              </span>
            </div>
          )}
          {label && (
            <span className="pointer-events-none absolute inset-x-0 bottom-0 flex items-center bg-gradient-to-t from-black/75 to-transparent px-2.5 pt-5 pb-2 text-[10px] font-semibold text-white">
              <span>{label}</span>
            </span>
          )}
        </>
      ) : (
        <span className="text-[10px] text-slate-500">{noImageText}</span>
      )}
    </div>
  )
}
