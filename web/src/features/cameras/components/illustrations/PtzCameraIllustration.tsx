import React from 'react'
import type { CameraIllustrationProps } from './types'
import { ILLUSTRATION_STATUS_COLORS } from './types'

export function PtzCameraIllustration({
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
        ariaLabel ?? `PTZ Camera Illustration (${status}${aiActive ? ', AI Active' : ''})`
      }
    >
      {/* ── 0. AI 激活状态雷达扩散波 ── */}
      {aiActive && !isOffline && (
        <g className="camera-svg-radar camera-svg-pulse">
          <circle
            cx="120"
            cy="84"
            r="38"
            fill="none"
            stroke="#38bdf8"
            strokeWidth="1.5"
            strokeDasharray="4 4"
            opacity="0.85"
          />
        </g>
      )}

      {/* ── 1. 顶部吊装弯臂 ── */}
      <path
        d="M 68 28 L 120 28 L 120 42"
        fill="none"
        stroke="currentColor"
        className="text-slate-800 dark:text-slate-200"
        strokeWidth="2.5"
        strokeLinecap="round"
      />

      {/* ── 2. 水平回转云台座 ── */}
      <rect
        x="96"
        y="42"
        width="48"
        height="12"
        rx="4"
        className="fill-white stroke-slate-800 dark:fill-slate-900 dark:stroke-slate-200"
        strokeWidth="2.5"
      />

      {/* ── 3. 俯仰球体 ── */}
      <circle
        cx="120"
        cy="86"
        r="32"
        className="fill-white stroke-slate-800 dark:fill-slate-900 dark:stroke-slate-200"
        strokeWidth="2.5"
      />

      {/* ── 4. 辅助传感器与状态灯 ── */}
      <circle cx="120" cy="65" r="2.5" fill="#94a3b8" />
      <circle
        cx="138"
        cy="68"
        r="2.2"
        fill={statusColor}
        className={status === 'online' ? 'animate-pulse' : ''}
      />

      {/* ── 5. 主光学变焦镜头 ── */}
      <g>
        <circle cx="120" cy="88" r="20" fill="#1e293b" />
        <circle cx="120" cy="88" r="14" fill="#334155" />
        <circle cx="120" cy="88" r="9" fill="#0f172a" />
        <circle cx="116" cy="84" r="2.8" fill="#ffffff" />
      </g>
    </svg>
  )
}
