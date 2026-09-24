import React, { useEffect, useRef } from 'react'

type CursorMode = 'default' | 'clickable' | 'input' | 'destructive'

interface RingConfig {
  width: string
  height: string
  borderRadius: string
  borderColor: string
  backgroundColor: string
  boxShadow: string
  dotOpacity: string
  dotBg: string
  dotShadow: string
}

const RING_CONFIGS: Record<CursorMode, RingConfig> = {
  default: {
    width: '30px',
    height: '30px',
    borderRadius: '9999px',
    borderColor: 'rgba(var(--accent-rgb, 59, 130, 246), 0.45)',
    backgroundColor: 'rgba(var(--accent-rgb, 59, 130, 246), 0.04)',
    boxShadow: '0 0 12px rgba(var(--accent-rgb, 59, 130, 246), 0.18)',
    dotOpacity: '1',
    dotBg: 'var(--accent)',
    dotShadow: '0 0 6px rgba(var(--accent-rgb, 59, 130, 246), 0.7)',
  },
  clickable: {
    width: '44px',
    height: '44px',
    borderRadius: '9999px',
    borderColor: 'var(--accent)',
    backgroundColor: 'var(--accent-soft)',
    boxShadow: '0 0 18px rgba(var(--accent-rgb, 59, 130, 246), 0.35)',
    dotOpacity: '1',
    dotBg: 'var(--accent)',
    dotShadow: '0 0 8px rgba(var(--accent-rgb, 59, 130, 246), 0.8)',
  },
  input: {
    width: '3px',
    height: '20px',
    borderRadius: '2px',
    borderColor: 'var(--accent)',
    backgroundColor: 'var(--accent)',
    boxShadow: '0 0 10px rgba(var(--accent-rgb, 59, 130, 246), 0.5)',
    dotOpacity: '0',
    dotBg: 'var(--accent)',
    dotShadow: '0 0 6px rgba(var(--accent-rgb, 59, 130, 246), 0.7)',
  },
  destructive: {
    width: '42px',
    height: '42px',
    borderRadius: '9999px',
    borderColor: 'var(--status-danger)',
    backgroundColor: 'rgba(var(--status-danger-rgb), 0.12)',
    boxShadow: '0 0 18px rgba(var(--status-danger-rgb), 0.4)',
    dotOpacity: '1',
    dotBg: 'var(--status-danger)',
    dotShadow: '0 0 8px rgba(var(--status-danger-rgb), 0.9)',
  },
}

export function CursorRing(): React.ReactElement | null {
  const dotRef = useRef<HTMLDivElement | null>(null)
  const ringRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    // 触控设备或开启无障碍减少动态效果时直接禁用
    if (
      typeof window === 'undefined' ||
      window.matchMedia('(pointer: coarse)').matches ||
      window.matchMedia('(prefers-reduced-motion: reduce)').matches
    ) {
      return
    }

    const dot = dotRef.current
    const ring = ringRef.current
    if (!dot || !ring) return

    let mouseX = -100
    let mouseY = -100
    let ringX = -100
    let ringY = -100
    let isVisible = false
    let isMouseDown = false
    let currentMode: CursorMode = 'default'
    let animId = 0
    let isMoving = false

    const ensureRendering = () => {
      if (!animId && !document.hidden && isVisible) {
        animId = requestAnimationFrame(render)
      }
    }

    const updateRingStyle = () => {
      const cfg = RING_CONFIGS[currentMode]
      ring.style.width = cfg.width
      ring.style.height = cfg.height
      ring.style.borderRadius = cfg.borderRadius
      ring.style.borderColor = cfg.borderColor
      ring.style.backgroundColor = cfg.backgroundColor
      ring.style.boxShadow = cfg.boxShadow
      dot.style.opacity = cfg.dotOpacity
      dot.style.backgroundColor = cfg.dotBg
      dot.style.boxShadow = cfg.dotShadow
    }

    const onPointerMove = (e: PointerEvent) => {
      mouseX = e.clientX
      mouseY = e.clientY

      // 核心激光点零延迟即时跟手
      dot.style.transform = `translate3d(${mouseX}px, ${mouseY}px, 0) translate(-50%, -50%)`

      if (!isVisible) {
        isVisible = true
        ringX = mouseX
        ringY = mouseY
        dot.style.opacity = currentMode === 'input' ? '0' : '1'
        ring.style.opacity = '1'
      }

      isMoving = true
      ensureRendering()
    }

    const onPointerDown = () => {
      isMouseDown = true
      ensureRendering()
    }

    const onPointerUp = () => {
      isMouseDown = false
      ensureRendering()
    }

    const onPointerLeave = () => {
      isVisible = false
      dot.style.opacity = '0'
      ring.style.opacity = '0'
      if (animId) {
        cancelAnimationFrame(animId)
        animId = 0
      }
    }

    const onPointerOver = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null
      if (!target) return

      const input = target.closest(
        'input[type=text], input[type=password], input[type=search], textarea',
      )
      if (input) {
        currentMode = 'input'
        updateRingStyle()
        ensureRendering()
        return
      }

      const destructive = target.closest(
        '[data-destructive="true"], .text-\\[var\\(--status-danger\\)\\]',
      )
      if (destructive) {
        currentMode = 'destructive'
        updateRingStyle()
        ensureRendering()
        return
      }

      const clickable = target.closest(
        'button, a, input[type=checkbox], input[type=radio], label, select, [role="button"], [role="tab"], .reticle-target, .nav-btn',
      )
      if (clickable) {
        currentMode = 'clickable'
        updateRingStyle()
        ensureRendering()
        return
      }

      if (currentMode !== 'default') {
        currentMode = 'default'
        updateRingStyle()
        ensureRendering()
      }
    }

    window.addEventListener('pointermove', onPointerMove, { passive: true })
    window.addEventListener('pointerdown', onPointerDown, { passive: true })
    window.addEventListener('pointerup', onPointerUp, { passive: true })
    document.addEventListener('pointerover', onPointerOver, { passive: true })
    document.documentElement.addEventListener('pointerleave', onPointerLeave, { passive: true })

    const render = () => {
      if (isVisible) {
        const dx = mouseX - ringX
        const dy = mouseY - ringY
        ringX += dx * 0.28
        ringY += dy * 0.28

        let scaleStr = 'scale(1)'
        if (isMouseDown) {
          scaleStr = 'scale(0.78)'
        } else if (currentMode === 'clickable') {
          scaleStr = 'scale(1.06)'
        }

        ring.style.transform = `translate3d(${ringX}px, ${ringY}px, 0) translate(-50%, -50%) ${scaleStr}`

        if (Math.abs(dx) < 0.1 && Math.abs(dy) < 0.1 && !isMouseDown) {
          isMoving = false
          animId = 0
          return
        }
      }

      animId = requestAnimationFrame(render)
    }

    const onVisibilityChange = () => {
      if (document.hidden) {
        if (animId) {
          cancelAnimationFrame(animId)
          animId = 0
        }
      } else if (isMoving && isVisible) {
        ensureRendering()
      }
    }
    document.addEventListener('visibilitychange', onVisibilityChange)

    return () => {
      if (animId) {
        cancelAnimationFrame(animId)
      }
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerdown', onPointerDown)
      window.removeEventListener('pointerup', onPointerUp)
      document.removeEventListener('pointerover', onPointerOver)
      document.documentElement.removeEventListener('pointerleave', onPointerLeave)
      document.removeEventListener('visibilitychange', onVisibilityChange)
    }
  }, [])

  return (
    <div className="pointer-events-none fixed inset-0 z-50 overflow-hidden select-none max-md:hidden">
      {/* 1. 外围环境物理跟随微光环 */}
      <div
        ref={ringRef}
        className="pointer-events-none fixed top-0 left-0 h-7 w-7 rounded-full border-[1.5px] border-[var(--accent)]/45 bg-[var(--accent)]/5 opacity-0 backdrop-blur-[0.5px] transition-[width,height,border-radius,border-color,background-color,box-shadow,opacity] duration-200 ease-out will-change-transform"
      />
      {/* 2. 核心激光点 (0 延迟即时瞄准) */}
      <div
        ref={dotRef}
        className="pointer-events-none fixed top-0 left-0 h-1 w-1 rounded-full bg-[var(--accent)] opacity-0 shadow-[0_0_6px_rgba(var(--accent-rgb),0.8)] transition-opacity duration-150 ease-out will-change-transform"
      />
    </div>
  )
}
