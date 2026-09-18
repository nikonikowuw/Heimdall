import { useCallback, useEffect, useRef, useState } from 'react'

export interface UseImageZoomPanOptions {
  minZoom?: number
  maxZoom?: number
  step?: number
  wheelStep?: number
  doubleClickZoom?: number
  enableKeyBindings?: boolean
  onToggleStatus?: () => void
  onToggleFullscreen?: () => void
}

export function clampZoom(val: number, min = 1, max = 5): number {
  return Math.min(max, Math.max(min, Number(val.toFixed(2))))
}

export function calcZoomIn(current: number, step = 0.3, max = 5): number {
  return clampZoom(current + step, 1, max)
}

export function calcZoomOut(current: number, step = 0.3, min = 1): number {
  return clampZoom(current - step, min, 5)
}

export function useImageZoomPan(
  containerRef: React.RefObject<HTMLElement | null>,
  options: UseImageZoomPanOptions = {},
) {
  const {
    minZoom = 1,
    maxZoom = 5,
    step = 0.3,
    wheelStep = 0.2,
    doubleClickZoom = 2.2,
    enableKeyBindings = true,
    onToggleStatus,
    onToggleFullscreen,
  } = options

  const [zoom, setZoom] = useState<number>(1)
  const [pan, setPan] = useState<{ x: number; y: number }>({ x: 0, y: 0 })
  const [isDragging, setIsDragging] = useState<boolean>(false)
  const dragStartRef = useRef<{ x: number; y: number }>({ x: 0, y: 0 })

  const resetZoom = useCallback(() => {
    setZoom(1)
    setPan({ x: 0, y: 0 })
  }, [])

  const zoomIn = useCallback(() => {
    setZoom((z) => calcZoomIn(z, step, maxZoom))
  }, [maxZoom, step])

  const zoomOut = useCallback(() => {
    setZoom((z) => {
      const next = calcZoomOut(z, step, minZoom)
      if (next <= minZoom) setPan({ x: 0, y: 0 })
      return next
    })
  }, [minZoom, step])

  // 快捷键支持 (+, -, 0, f, Space) 智能避开交互输入框
  useEffect(() => {
    if (!enableKeyBindings) return

    const handleKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      const isInteractive =
        target &&
        (target.tagName === 'BUTTON' ||
          target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.tagName === 'SELECT' ||
          target.tagName === 'A')

      if (e.key === '+' || e.key === '=') {
        e.preventDefault()
        zoomIn()
      } else if (e.key === '-' || e.key === '_') {
        e.preventDefault()
        zoomOut()
      } else if (e.key === '0') {
        e.preventDefault()
        resetZoom()
      } else if ((e.key === 'f' || e.key === 'F') && !isInteractive && onToggleFullscreen) {
        e.preventDefault()
        onToggleFullscreen()
      } else if (e.key === ' ' && !isInteractive && onToggleStatus) {
        e.preventDefault()
        onToggleStatus()
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [enableKeyBindings, zoomIn, zoomOut, resetZoom, onToggleFullscreen, onToggleStatus])

  // 滚轮缩放监听 (passive: false 允许阻止视口滚动)
  useEffect(() => {
    const container = containerRef.current
    if (!container) return

    const handleWheel = (e: WheelEvent) => {
      e.preventDefault()
      if (e.deltaY < 0) {
        setZoom((z) => clampZoom(z + wheelStep, minZoom, maxZoom))
      } else {
        setZoom((z) => {
          const next = clampZoom(z - wheelStep, minZoom, maxZoom)
          if (next <= minZoom) setPan({ x: 0, y: 0 })
          return next
        })
      }
    }

    container.addEventListener('wheel', handleWheel, { passive: false })
    return () => container.removeEventListener('wheel', handleWheel)
  }, [containerRef, maxZoom, minZoom, wheelStep])

  const handleMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (zoom <= minZoom) return
      setIsDragging(true)
      dragStartRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y }
    },
    [zoom, minZoom, pan.x, pan.y],
  )

  const handleMouseMove = useCallback(
    (e: React.MouseEvent) => {
      if (!isDragging || zoom <= minZoom) return
      setPan({
        x: e.clientX - dragStartRef.current.x,
        y: e.clientY - dragStartRef.current.y,
      })
    },
    [isDragging, zoom, minZoom],
  )

  const handleMouseUp = useCallback(() => {
    setIsDragging(false)
  }, [])

  const handleDoubleClick = useCallback(() => {
    if (zoom > minZoom) {
      resetZoom()
    } else {
      setZoom(doubleClickZoom)
    }
  }, [zoom, minZoom, resetZoom, doubleClickZoom])

  return {
    zoom,
    pan,
    isDragging,
    zoomIn,
    zoomOut,
    resetZoom,
    setZoom,
    setPan,
    dragProps: {
      onMouseDown: handleMouseDown,
      onMouseMove: handleMouseMove,
      onMouseUp: handleMouseUp,
      onMouseLeave: handleMouseUp,
      onDoubleClick: handleDoubleClick,
    },
    containerCursorClass:
      zoom > minZoom ? (isDragging ? 'cursor-grabbing' : 'cursor-grab') : 'cursor-zoom-in',
    transformStyle: {
      transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
      transformOrigin: 'center center',
      transition: isDragging ? 'none' : 'transform 120ms ease-out',
    } as React.CSSProperties,
  }
}
