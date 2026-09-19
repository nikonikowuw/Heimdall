import React from 'react'
import type { CameraIllustrationProps } from './types'
import { ILLUSTRATION_STATUS_COLORS } from './types'

export function BulletCameraIllustration({
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
        ariaLabel ?? `Bullet Camera Illustration (${status}${aiActive ? ', AI Active' : ''})`
      }
    >
      {/* ── 0. AI 扫描激活波纹 ── */}
      {aiActive && !isOffline && (
        <g className="camera-svg-scan camera-svg-pulse">
          <circle
            cx="120"
            cy="69"
            r="30"
            fill="none"
            stroke="#38bdf8"
            strokeWidth="1.5"
            strokeDasharray="4 4"
            opacity="0.85"
          />
        </g>
      )}

      {/* ── 1. 极简 T 型金属支架 ── */}
      <g stroke="currentColor" className="text-slate-800 dark:text-slate-200">
        <line x1="120" y1="106" x2="120" y2="130" strokeWidth="2.5" strokeLinecap="round" />
        <line x1="98" y1="130" x2="142" y2="130" strokeWidth="2.5" strokeLinecap="round" />
      </g>

      {/* ── 2. 现代圆角摄像头机身 ── */}
      <rect
        x="68"
        y="32"
        width="104"
        height="74"
        rx="26"
        className="fill-white stroke-slate-800 dark:fill-slate-900 dark:stroke-slate-200"
        strokeWidth="2.5"
      />

      {/* ── 3. 顶部传感器与状态指示灯 ── */}
      {/* 居中光敏/麦克风点 */}
      <circle cx="120" cy="45" r="2.5" fill="#94a3b8" />
      {/* 状态指示 LED */}
      <circle
        cx="152"
        cy="46"
        r="2.5"
        fill={statusColor}
        className={status === 'online' ? 'animate-pulse' : ''}
      />

      {/* ── 4. 核心同心多层光学镜组 ── */}
      <g>
        {/* 外圈黑圈 */}
        <circle cx="120" cy="69" r="23" fill="#1e293b" />
        {/* 内圈镜身 */}
        <circle cx="120" cy="69" r="17" fill="#334155" />
        {/* 瞳孔深邃黑芯 */}
        <circle cx="120" cy="69" r="11" fill="#0f172a" />
        {/* 镜头圆形高光光斑 */}
        <circle cx="116" cy="64" r="3.2" fill="#ffffff" />
      </g>
    </svg>
  )
}
