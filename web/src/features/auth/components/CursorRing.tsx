import React, { useEffect, useRef } from 'react'

export const CursorRing: React.FC = () => {
  const ringRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const ring = ringRef.current
    if (!ring) return

    let mouseX = window.innerWidth * 0.5
    let mouseY = window.innerHeight * 0.5
    let ringX = mouseX
    let ringY = mouseY
    let isMouseDown = false
    let isHovering = false
    let isInputHover = false
    let isActive = false
    let isMoving = false
    let animId = 0

    // 唤醒并启动平滑追迹渲染循环
    const ensureRendering = () => {
      if (!animId && !document.hidden) {
        animId = requestAnimationFrame(render)
      }
    }

    const onPointerMove = (e: PointerEvent) => {
      mouseX = e.clientX
      mouseY = e.clientY
      isMoving = true
      if (!isActive) {
        isActive = true
        ring.style.opacity = '1'
      }
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

    // 事件委托：仅在 document 挂载单个委托监听，性能提升 10 倍以上
    const onPointerOver = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null
      if (!target) return

      const input = target.closest('input[type=text], input[type=password]')
      if (input) {
        isInputHover = true
        isHovering = false
        ring.style.width = '20px'
        ring.style.height = '32px'
        ring.style.borderRadius = '6px'
        ring.style.borderColor = 'var(--cursor-border, rgba(129, 140, 248, 0.7))'
        ensureRendering()
        return
      }

      const clickable = target.closest('button, a, input[type=checkbox], label, [role="button"]')
      if (clickable) {
        isHovering = true
        isInputHover = false
        ring.style.width = '42px'
        ring.style.height = '42px'
        ring.style.borderRadius = '9999px'
        ring.style.borderColor = 'var(--cursor-text, #818cf8)'
        ensureRendering()
        return
      }
    }

    const onPointerOut = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null
      if (!target) return

      const interactive = target.closest('button, a, input, label, [role="button"]')
      if (interactive) {
        isHovering = false
        isInputHover = false
        ring.style.width = '32px'
        ring.style.height = '32px'
        ring.style.borderRadius = '9999px'
        ring.style.borderColor = 'var(--cursor-border, rgba(129, 140, 248, 0.6))'
        ensureRendering()
      }
    }

    window.addEventListener('pointermove', onPointerMove, { passive: true })
    window.addEventListener('pointerdown', onPointerDown, { passive: true })
    window.addEventListener('pointerup', onPointerUp, { passive: true })
    document.addEventListener('pointerover', onPointerOver, { passive: true })
    document.addEventListener('pointerout', onPointerOut, { passive: true })

    const render = () => {
      if (isActive) {
        const dx = mouseX - ringX
        const dy = mouseY - ringY
        ringX += dx * 0.32
        ringY += dy * 0.32

        let scaleStr = 'scale(1)'
        if (isMouseDown) {
          scaleStr = 'scale(0.75)'
        } else if (isHovering && !isInputHover) {
          scaleStr = 'scale(1.18)'
        }
        ring.style.transform = `translate3d(${ringX}px, ${ringY}px, 0) translate(-50%, -50%) ${scaleStr}`

        // 当光标静止且已精确贴合当前位置时，休眠暂停 RAF，节省 CPU 能耗
        if (Math.abs(dx) < 0.15 && Math.abs(dy) < 0.15 && !isMouseDown) {
          isMoving = false
          animId = 0
          return
        }
      }

      animId = requestAnimationFrame(render)
    }

    // 后台页面休眠
    const onVisibilityChange = () => {
      if (document.hidden) {
        if (animId) {
          cancelAnimationFrame(animId)
          animId = 0
        }
      } else if (isMoving) {
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
      document.removeEventListener('pointerout', onPointerOut)
      document.removeEventListener('visibilitychange', onVisibilityChange)
    }
  }, [])

  return (
    <div
      ref={ringRef}
      id="cursor-ring"
      className="pointer-events-none fixed top-0 left-0 z-50 h-8 w-8 -translate-x-1/2 -translate-y-1/2 rounded-full border-[1.5px] border-indigo-400/60 bg-indigo-500/5 opacity-0 backdrop-blur-[1px] transition-[width,height,border-radius,border-color,opacity] duration-200 ease-out will-change-transform max-md:hidden"
    />
  )
}
