import React from 'react'
import type { CameraIllustrationProps } from './types'
import { ILLUSTRATION_STATUS_COLORS } from './types'

export function DomeCameraIllustration({
  status,
  aiActive = false,
  ariaLabel,
  className = '',
  width = '100%',
  height = '100%',
}: CameraIllustrationProps): React.ReactElement {
  const isOffline = status === 'offline'
  const statusColor = ILLUSTRATION_STATUS_COLORS[status]

  return (
    <svg
      viewBox="0 0 240 160"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={`camera-svg-root ${isOffline ? 'camera-svg-offline' : ''} overflow-visible select-none ${className}`}
      style={{ width, height }}
      role="img"
      data-camera-status={status}
      aria-label={
        ariaLabel ?? `Dome Camera Illustration (${status}${aiActive ? ', AI Active' : ''})`
      }
    >
      {/* ── 0. AI 激活向下扫描波 ── */}
      {aiActive && !isOffline && (
        <g className="camera-svg-pulse">
          <circle
            cx="120"
            cy="75"
            r="28"
            fill="none"
            stroke="#38bdf8"
            strokeWidth="1.5"
            strokeDasharray="4 4"
            opacity="0.85"
          />
        </g>
      )}

      {/* ── 1. 吸顶安装顶板 ── */}
      <rect
        x="72"
        y="32"
        width="96"
        height="8"
        rx="4"
        className="fill-white stroke-slate-800 dark:fill-slate-900 dark:stroke-slate-200"
        strokeWidth="2.5"
      />

      {/* ── 2. 半球型罩体 ── */}
      <path
        d="M 76 40 C 76 86 94 112 120 112 C 146 112 164 86 164 40 Z"
        className="fill-white stroke-slate-800 dark:fill-slate-900 dark:stroke-slate-200"
        strokeWidth="2.5"
      />

      {/* ── 3. 状态指示微灯 ── */}
      <circle
        cx="120"
        cy="48"
        r="2.5"
        fill={statusColor}
        className={status === 'online' ? 'animate-pulse' : ''}
      />

      {/* ── 4. 核心同心光学镜片 ── */}
      <g>
        <circle cx="120" cy="75" r="22" fill="#1e293b" />
        <circle cx="120" cy="75" r="16" fill="#334155" />
        <circle cx="120" cy="75" r="10" fill="#0f172a" />
        <circle cx="116" cy="70" r="3.2" fill="#ffffff" />
      </g>
    </svg>
  )
}
