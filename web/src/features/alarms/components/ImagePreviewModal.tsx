import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Download, ExternalLink, Maximize2, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '../../../hooks/use-dismiss-stack'
import {
  calculateFittedImageRect,
  deriveDownloadFilename,
  getBBoxStyle,
  parseTargetBBoxes,
  type FittedImageRect,
} from '../utils'

export interface ImagePreviewModalProps {
  src: string
  title?: string
  subtitle?: string
  alt?: string
  filename?: string
  bboxJson?: string | null
  onClose: () => void
}

function triggerDownload(href: string, filename: string, isFallback = false): void {
  const link = document.createElement('a')
  link.href = href
  link.download = filename
  if (isFallback) {
    link.target = '_blank'
    link.rel = 'noreferrer'
  }
  document.body.appendChild(link)
  link.click()
  document.body.removeChild(link)
}

export function ImagePreviewModal({
  src,
  title,
  subtitle,
  alt = 'Image Preview',
  filename,
  bboxJson,
  onClose,
}: ImagePreviewModalProps): React.ReactElement | null {
  const { t } = useTranslation('alarm')

  // 设定优先级 20，确保在复核弹窗等其他 Modal 之上优先响应 ESC 退出
  useDismissStack(true, onClose, { priority: 20 })

  const downloadFilename = useMemo(
    () => deriveDownloadFilename(src, filename, title),
    [src, filename, title],
  )
  const imageContainerRef = useRef<HTMLDivElement>(null)
  const imageRef = useRef<HTMLImageElement>(null)
  const [imgRect, setImgRect] = useState<FittedImageRect | null>(null)
  const targetBBoxes = parseTargetBBoxes(bboxJson ?? undefined)

  const updateImageRect = useCallback(() => {
    const container = imageContainerRef.current
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
    const container = imageContainerRef.current
    if (!container || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(updateImageRect)
    observer.observe(container)
    return () => observer.disconnect()
  }, [updateImageRect])

  const handleDownload = async (e: React.MouseEvent): Promise<void> => {
    e.preventDefault()
    e.stopPropagation()
    try {
      const res = await fetch(src)
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const blob = await res.blob()
      const blobUrl = URL.createObjectURL(blob)
      triggerDownload(blobUrl, downloadFilename)
      URL.revokeObjectURL(blobUrl)
    } catch {
      triggerDownload(src, downloadFilename, true)
    }
  }

  if (typeof document === 'undefined') return null

  return createPortal(
    <div
      className="animate-in fade-in fixed inset-0 z-[100] flex flex-col items-center justify-center bg-black/90 p-4 backdrop-blur-md duration-150"
      onClick={onClose}
    >
      {/* 浮动工具栏 */}
      <div
        className="absolute top-4 right-4 left-4 flex items-center justify-between gap-3 rounded-2xl border border-white/10 bg-black/60 px-4 py-3 backdrop-blur-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-white/10 text-white">
            <Maximize2 className="h-4 w-4" />
          </div>
          <div className="min-w-0">
            {title && <h4 className="truncate text-xs font-semibold text-white">{title}</h4>}
            {subtitle && <p className="truncate font-mono text-[11px] text-zinc-400">{subtitle}</p>}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          <button
            type="button"
            onClick={handleDownload}
            className="flex items-center gap-1 rounded-xl border border-white/10 bg-white/5 px-2.5 py-1.5 text-xs text-zinc-300 transition-all hover:bg-white/15 hover:text-white"
            title={`${t('modal.download')} (${downloadFilename})`}
          >
            <Download className="h-3.5 w-3.5" />
          </button>
          <a
            href={src}
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-1 rounded-xl border border-white/10 bg-white/5 px-2.5 py-1.5 text-xs text-zinc-300 transition-all hover:bg-white/15 hover:text-white"
            title={t('modal.openOriginal')}
          >
            <ExternalLink className="h-3.5 w-3.5" />
          </a>
          <button
            type="button"
            onClick={onClose}
            className="rounded-xl border border-white/10 bg-white/5 p-1.5 text-zinc-400 transition-all hover:bg-rose-500/20 hover:text-rose-400"
            title={`${t('modal.close')} (Esc)`}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* 居中大图 */}
      <div
        ref={imageContainerRef}
        className="relative flex max-h-[82vh] max-w-[90vw] items-center justify-center overflow-hidden rounded-2xl shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <img
          ref={imageRef}
          src={src}
          alt={alt}
          className="max-h-[82vh] max-w-[90vw] rounded-2xl border border-white/15 object-contain shadow-2xl transition-transform duration-200"
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
                ? `${t('card.face')} ${(targetBBoxes.face.qualityScore * 100).toFixed(0)}%`
                : t('card.face')}
            </span>
          </div>
        )}
      </div>

      {/* 底部按键提示 */}
      <div className="absolute bottom-3 text-center text-[10px] text-zinc-500 select-none">
        {t('modal.escHint')}
      </div>
    </div>,
    document.body,
  )
}
