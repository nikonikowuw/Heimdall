import React, { useState } from 'react'
import {
  Activity,
  ArrowRight,
  Check,
  Copy,
  Hexagon,
  Layers,
  Pencil,
  Radio,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  Video,
} from 'lucide-react'
import { motion } from 'motion/react'
import { getProbeBadge } from '@/features/cameras'
import { motionTokens } from '@/lib/motionTokens'
import type { Camera, DetectionRule, TaskConfigDto } from '@/types'

export interface TaskCameraCardProps {
  camera: Camera
  config?: TaskConfigDto
  onToggleArm: () => void
  onConfigure: () => void
  onDelete: () => void
  t: (key: string, options?: Record<string, unknown>) => string
}

export function TaskCameraCard({
  camera,
  config,
  onToggleArm,
  onConfigure,
  onDelete,
  t,
}: TaskCameraCardProps): React.ReactElement {
  const isArmed = config?.desiredEnabled ?? false
  const rules: DetectionRule[] = config?.rules ?? []
  const rulesCount = rules.length
  const roiCount = rules.filter((r) => r.role === 'roi').length
  const lineCount = rules.filter((r) => r.role === 'line').length
  const maskCount = rules.filter((r) => r.role === 'mask').length
  const isMotionGateEco = config?.motionGate?.enabled ?? false

  const [copied, setCopied] = useState(false)
  const probeBadge = getProbeBadge(camera.lastProbeStatus, t)

  const handleCopyRtsp = async (e: React.MouseEvent) => {
    e.stopPropagation()
    try {
      await navigator.clipboard.writeText(camera.rtspUrl)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      // 容错处理
    }
  }

  return (
    <motion.div
      role="button"
      tabIndex={0}
      onClick={onConfigure}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onConfigure()
        }
      }}
      whileHover={{ y: -4 }}
      transition={{ duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }}
      className={`group frosted-glass relative flex cursor-pointer flex-col overflow-hidden rounded-2xl border p-4 text-left shadow-xs transition-all select-none ${
        isArmed
          ? 'border-[var(--accent)]/50 shadow-md hover:border-[var(--accent)] hover:shadow-lg'
          : 'border-[var(--border)] hover:border-[var(--accent)]/40 hover:shadow-md'
      }`}
    >
      {/* 顶部武装状态外发光微氛围 */}
      {isArmed && (
        <div className="pointer-events-none absolute -top-12 -right-12 h-28 w-28 rounded-full bg-[var(--accent)]/15 blur-2xl transition-all group-hover:bg-[var(--accent)]/25" />
      )}

      {/* 1. 头部设备信息与快捷操作 */}
      <div className="relative z-10 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-3">
          {/* 设备图标 + 状态指示呼吸灯 */}
          <div className="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] shadow-2xs">
            <Video className="h-5 w-5 text-[var(--accent)]" />
            <span
              className={`absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-[var(--bg-surface-solid)] ${probeBadge.dotClass}`}
            />
          </div>

          {/* 标题与通道标识 */}
          <div className="min-w-0">
            <h4 className="truncate text-sm font-bold text-[var(--text-primary)] transition-colors group-hover:text-[var(--accent)]">
              {camera.name}
            </h4>
            <div className="mt-0.5 flex items-center gap-1.5 font-mono text-xs text-[var(--text-muted)]">
              <span className="py-0.2 rounded bg-[var(--bg-secondary)] px-1 font-semibold">
                RTSP
              </span>
              <span className="truncate">{camera.cameraId}</span>
            </div>
          </div>
        </div>

        {/* 右侧动作群：布防状态胶囊 + 快速编辑/删除 */}
        <div className="flex shrink-0 items-center gap-1.5">
          {/* 布防总开关 */}
          <motion.button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onToggleArm()
            }}
            whileTap={{ scale: 0.94 }}
            className={`flex items-center gap-1.5 rounded-xl border px-2.5 py-1 text-xs font-semibold transition-all ${
              isArmed
                ? 'border-rose-500/30 bg-rose-500/15 text-rose-500 shadow-xs hover:bg-rose-500/25'
                : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] hover:border-[var(--accent)]/40 hover:text-[var(--text-primary)]'
            }`}
            title={isArmed ? t('status.armed') : t('status.disarmed')}
          >
            {isArmed ? (
              <>
                <ShieldAlert className="h-3.5 w-3.5" />
                <span>{t('status.armed')}</span>
              </>
            ) : (
              <>
                <ShieldCheck className="h-3.5 w-3.5" />
                <span>{t('status.disarmed')}</span>
              </>
            )}
          </motion.button>

          {/* 进入画板配置 */}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onConfigure()
            }}
            title={t('actions.configureRules', { defaultValue: '配置布防规则' })}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>

          {/* 删除布防任务 */}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation()
              onDelete()
            }}
            title={t('deleteTask', { defaultValue: '删除布防任务' })}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] transition-colors hover:border-rose-500/40 hover:bg-rose-500/15 hover:text-rose-500"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* 2. 核心视窗与空间几何布防微缩舞台 */}
      <div className="relative mt-3 aspect-[2.1/1] w-full overflow-hidden rounded-xl border border-[var(--border)] bg-black/90 shadow-inner">
        {/* 精密十字十字线准星底纹 */}
        <div
          className="pointer-events-none absolute inset-0 opacity-20"
          style={{
            backgroundImage:
              'radial-gradient(circle, rgba(255,255,255,0.2) 1px, transparent 1px), linear-gradient(to right, rgba(255,255,255,0.05) 1px, transparent 1px), linear-gradient(to bottom, rgba(255,255,255,0.05) 1px, transparent 1px)',
            backgroundSize: '16px 16px',
          }}
        />

        {/* 视口四角取景标尺 L-brackets */}
        <div className="pointer-events-none absolute top-1.5 left-1.5 h-2 w-2 border-t-2 border-l-2 border-white/40" />
        <div className="pointer-events-none absolute top-1.5 right-1.5 h-2 w-2 border-t-2 border-r-2 border-white/40" />
        <div className="pointer-events-none absolute bottom-1.5 left-1.5 h-2 w-2 border-b-2 border-l-2 border-white/40" />
        <div className="pointer-events-none absolute right-1.5 bottom-1.5 h-2 w-2 border-r-2 border-b-2 border-white/40" />

        {/* 几何规则动态微缩 SVG 绘制 */}
        {rulesCount > 0 ? (
          <svg
            className="absolute inset-0 h-full w-full"
            viewBox="0 0 100 100"
            preserveAspectRatio="none"
          >
            {(() => {
              const ROI_PALETTES = [
                { stroke: '#06b6d4', fill: 'rgba(6, 182, 212, 0.24)' },
                { stroke: '#f59e0b', fill: 'rgba(245, 158, 11, 0.24)' },
                { stroke: '#a855f7', fill: 'rgba(168, 85, 247, 0.24)' },
                { stroke: '#f43f5e', fill: 'rgba(244, 63, 94, 0.24)' },
                { stroke: '#6366f1', fill: 'rgba(99, 102, 241, 0.24)' },
              ]
              let roiIdx = 0

              return rules.map((rule, idx) => {
                if (rule.role === 'roi' && rule.points.length >= 3) {
                  const pts = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')
                  const theme = ROI_PALETTES[roiIdx++ % ROI_PALETTES.length]
                  return (
                    <polygon
                      key={idx}
                      points={pts}
                      fill={theme.fill}
                      stroke={theme.stroke}
                      strokeWidth="1.8"
                      vectorEffect="non-scaling-stroke"
                    />
                  )
                }
                if (rule.role === 'mask' && rule.points.length >= 3) {
                  const pts = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')
                  return (
                    <polygon
                      key={idx}
                      points={pts}
                      fill="rgba(15, 23, 42, 0.65)"
                      stroke="#94a3b8"
                      strokeWidth="1.8"
                      vectorEffect="non-scaling-stroke"
                    />
                  )
                }
                if (rule.role === 'line' && rule.points.length >= 2) {
                  const p1 = rule.points[0]
                  const p2 = rule.points[1]
                  return (
                    <line
                      key={idx}
                      x1={`${p1.x * 100}`}
                      y1={`${p1.y * 100}`}
                      x2={`${p2.x * 100}`}
                      y2={`${p2.y * 100}`}
                      stroke="#10b981"
                      strokeWidth="2"
                      strokeLinecap="round"
                      vectorEffect="non-scaling-stroke"
                    />
                  )
                }
                return null
              })
            })()}
          </svg>
        ) : (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 text-[var(--text-muted)]">
            <Hexagon className="h-5 w-5 animate-pulse text-cyan-400 opacity-40" />
            <span className="font-mono text-xs tracking-wide opacity-70">
              {t('card.noRulesPlaceholder')}
            </span>
          </div>
        )}

        {/* 视口左上角：规格与编码 */}
        <div className="absolute top-2 left-2 z-10 flex items-center gap-1 rounded bg-black/75 px-2 py-0.5 font-mono text-[11px] text-white/90 backdrop-blur-xs">
          <span className="font-semibold text-cyan-400">
            {camera.lastCodec?.toUpperCase() || 'H264'}
          </span>
          <span className="opacity-40">/</span>
          <span>{camera.lastWidth ? `${camera.lastWidth}x${camera.lastHeight}` : '1080P'}</span>
        </div>

        {/* 视口右上角：FPS 实时帧率 */}
        <div className="absolute top-2 right-2 z-10 flex items-center gap-1 rounded bg-black/75 px-2 py-0.5 font-mono text-[11px] text-emerald-400 backdrop-blur-xs">
          <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />
          <span>{camera.lastFps ? camera.lastFps.toFixed(1) : '25.0'} FPS</span>
        </div>

        {/* 视口左下角：规则类型徽标 */}
        {rulesCount > 0 && (
          <div className="absolute bottom-2 left-2 z-10 flex items-center gap-1 font-mono text-xs">
            {roiCount > 0 && (
              <span className="rounded bg-cyan-500/20 px-1.5 py-0.5 font-semibold text-cyan-300 backdrop-blur-xs">
                {roiCount} ROI
              </span>
            )}
            {lineCount > 0 && (
              <span className="rounded bg-emerald-500/20 px-1.5 py-0.5 font-semibold text-emerald-300 backdrop-blur-xs">
                {lineCount} LINE
              </span>
            )}
            {maskCount > 0 && (
              <span className="rounded bg-rose-500/20 px-1.5 py-0.5 font-semibold text-rose-300 backdrop-blur-xs">
                {maskCount} MASK
              </span>
            )}
          </div>
        )}

        {/* 悬停微动效：进入画板浮动引导 */}
        <div className="absolute right-2 bottom-2 z-10 flex items-center gap-1 rounded-md bg-[var(--accent)]/90 px-2 py-0.5 text-xs font-medium text-white opacity-0 shadow-md backdrop-blur-xs transition-all duration-200 group-hover:opacity-100">
          <span>{t('card.enterStudioHint')}</span>
        </div>
      </div>

      {/* 3. 关键性能与管线指标胶囊 */}
      <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
        {/* 规则总数 */}
        <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1.5">
          <span className="flex items-center gap-1.5 text-[var(--text-secondary)]">
            <Hexagon className="h-3.5 w-3.5 text-cyan-500" />
            <span>{t('card.geometryRules')}</span>
          </span>
          <span className="font-semibold text-cyan-500">
            {t('card.rulesCount', { count: rulesCount })}
          </span>
        </div>

        {/* 运动门控 */}
        <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1.5">
          <span className="flex items-center gap-1.5 text-[var(--text-secondary)]">
            <Activity className="h-3.5 w-3.5 text-emerald-500" />
            <span>{t('card.motionGate')}</span>
          </span>
          <span
            className={`font-semibold ${isMotionGateEco ? 'text-emerald-500' : 'text-amber-500'}`}
          >
            {isMotionGateEco ? t('card.motionGateEco') : t('card.motionGateAlways')}
          </span>
        </div>
      </div>

      {/* 4. RTSP 直通流单行地址与一键复制 */}
      <div className="mt-2 flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1.5 font-mono text-xs">
        <div className="flex min-w-0 items-center gap-1.5 text-[var(--text-secondary)]">
          <Radio className="h-3.5 w-3.5 shrink-0 text-[var(--text-muted)]" />
          <span className="shrink-0 text-[var(--text-muted)]">{t('card.rtspDirect')}</span>
          <span className="truncate text-[var(--text-primary)]" title={camera.rtspUrl}>
            {camera.rtspUrl}
          </span>
        </div>
        <button
          type="button"
          onClick={handleCopyRtsp}
          className="ml-2 flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-xs text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
          title={t('card.copyRtsp')}
        >
          {copied ? (
            <>
              <Check className="h-3 w-3 text-emerald-500" />
              <span className="text-emerald-500">{t('card.copied')}</span>
            </>
          ) : (
            <Copy className="h-3 w-3" />
          )}
        </button>
      </div>

      {/* 5. 底部状态与主行动呼应栏 */}
      <div className="mt-3 flex items-center justify-between border-t border-[var(--border)] pt-2.5">
        <span
          className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium ${probeBadge.badgeBg}`}
        >
          <span className={`h-1.5 w-1.5 rounded-full ${probeBadge.dotClass}`} />
          <span>{probeBadge.text}</span>
        </span>

        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation()
            onConfigure()
          }}
          className="flex items-center gap-1 rounded-xl bg-[var(--accent-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--accent)] shadow-2xs transition-all duration-200 hover:bg-[var(--accent)] hover:text-white"
        >
          <Layers className="h-3.5 w-3.5" />
          <span>{t('card.enterRules')}</span>
          <ArrowRight className="h-3 w-3 transition-transform duration-200 group-hover:translate-x-0.5" />
        </button>
      </div>
    </motion.div>
  )
}
