import React, { useState } from 'react'
import {
  Activity,
  ArrowLeftRight,
  Check,
  Copy,
  Hexagon,
  Layers,
  Pencil,
  Radio,
  ShieldCheck,
  Trash2,
  TriangleAlert,
  Video,
} from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { getProbeBadge } from '@/features/cameras'
import { motionTokens } from '@/lib/motionTokens'
import { copyToClipboard } from '@/lib/utils'
import type { Camera, DetectionRule, StreamMode, TaskConfigDto } from '@/types'
import { summarizeInstanceApply, unappliedNoticeLines } from '../applyState'

export interface TaskCameraCardProps {
  camera: Camera
  config?: TaskConfigDto
  onToggleArm: () => void
  onConfigure: () => void
  onStreamModeChange?: (mode: StreamMode) => void
  onDelete: () => void
  t: (key: string, options?: Record<string, unknown>) => string
}

function getPipelineRuntimeStatus(
  actualStatus: number,
  t: (key: string, opts?: { defaultValue?: string }) => string,
): { label: string; className: string } {
  switch (actualStatus) {
    case 1:
      return {
        label: t('card.pipelineStarting', { defaultValue: '启动中' }),
        className: 'text-[var(--accent-amber)]',
      }
    case 2:
      return {
        label: t('card.pipelineRunning', { defaultValue: '运行中' }),
        className: 'text-[var(--accent-green)]',
      }
    case 3:
    case 4:
      return {
        label: t('card.pipelineDegraded', { defaultValue: '重连中' }),
        className: 'text-[var(--accent-amber)]',
      }
    case 5:
      return {
        label: t('card.pipelineError', { defaultValue: '异常' }),
        className: 'text-[var(--destructive)]',
      }
    default:
      return {
        label: t('card.pipelineStopped', { defaultValue: '已停止' }),
        className: 'text-[var(--text-muted)]',
      }
  }
}

const ROI_PALETTES = [
  { stroke: '#06b6d4', fill: 'rgba(6, 182, 212, 0.24)' },
  { stroke: '#f59e0b', fill: 'rgba(245, 158, 11, 0.24)' },
  { stroke: '#a855f7', fill: 'rgba(168, 85, 247, 0.24)' },
  { stroke: '#f43f5e', fill: 'rgba(244, 63, 94, 0.24)' },
  { stroke: '#6366f1', fill: 'rgba(99, 102, 241, 0.24)' },
]

const STREAM_MODE_CONFIG: Record<
  StreamMode,
  { labelKey: string; defaultLabel: string; className: string }
> = {
  main: {
    labelKey: 'cardStream.main',
    defaultLabel: '主码流',
    className: 'border-blue-500/30 bg-blue-500/10 text-[var(--accent)]',
  },
  sub: {
    labelKey: 'cardStream.sub',
    defaultLabel: '子码流',
    className: 'border-amber-500/30 bg-amber-500/10 text-[var(--accent-amber)]',
  },
  auto: {
    labelKey: 'cardStream.auto',
    defaultLabel: '自动',
    className:
      'border-black/10 dark:border-white/10 bg-black/5 dark:bg-white/5 text-[var(--text-secondary)]',
  },
}

function renderPreviewRuleShape(
  rule: DetectionRule,
  idx: number,
  cameraId: string,
  roiIdx: number,
): React.ReactElement | null {
  if (rule.role === 'roi' && rule.points.length >= 3) {
    const pts = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')
    const theme = ROI_PALETTES[roiIdx % ROI_PALETTES.length]
    return (
      <polygon
        key={idx}
        points={pts}
        fill={theme.fill}
        stroke={theme.stroke}
        strokeWidth="1.6"
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
        fill={`url(#card-mask-hatch-${cameraId})`}
        stroke="rgba(244, 63, 94, 0.85)"
        strokeWidth="1.5"
        strokeDasharray="3 2"
        vectorEffect="non-scaling-stroke"
      />
    )
  }
  if (rule.role === 'line' && rule.points.length >= 2) {
    const p1 = rule.points[0]
    const p2 = rule.points[1]
    return (
      <g key={idx}>
        <line
          x1={`${p1.x * 100}`}
          y1={`${p1.y * 100}`}
          x2={`${p2.x * 100}`}
          y2={`${p2.y * 100}`}
          stroke="#10b981"
          strokeWidth="2"
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
        />
        <circle cx={`${p1.x * 100}`} cy={`${p1.y * 100}`} r="1.5" fill="#34d399" />
        <circle cx={`${p2.x * 100}`} cy={`${p2.y * 100}`} r="1.5" fill="#34d399" />
      </g>
    )
  }
  if (rule.role === 'precrop' && rule.points.length >= 2) {
    const pts = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')
    return (
      <polygon
        key={idx}
        points={pts}
        fill="rgba(132, 204, 22, 0.16)"
        stroke="#84cc16"
        strokeWidth="1.6"
        strokeDasharray="3 3"
        vectorEffect="non-scaling-stroke"
      />
    )
  }
  return null
}

function nextStreamMode(current: StreamMode | undefined): StreamMode {
  if (current === 'main') return 'sub'
  if (current === 'sub') return 'auto'
  return 'main'
}

export function TaskCameraCard({
  camera,
  config,
  onToggleArm,
  onConfigure,
  onStreamModeChange,
  onDelete,
  t,
}: TaskCameraCardProps): React.ReactElement {
  const reduceMotion = useReducedMotion()
  const isArmed = config?.desiredEnabled ?? false
  const rules: DetectionRule[] = config?.rules ?? []
  const rulesCount = rules.length
  const roiCount = rules.filter((r) => r.role === 'roi').length
  const lineCount = rules.filter((r) => r.role === 'line').length
  const maskCount = rules.filter((r) => r.role === 'mask').length
  const precropCount = rules.filter((r) => r.role === 'precrop').length
  const isMotionGateEco = config?.motionGate?.enabled ?? false
  const algorithmInstances = config?.algorithmInstances ?? []
  const enabledInstances = algorithmInstances.filter((instance) => instance.enabled)
  const primaryInstance = enabledInstances[0] ?? algorithmInstances[0]
  const algorithmId = primaryInstance?.algorithmId ?? config?.algorithmId ?? ''
  const analysisFps = primaryInstance?.analysisFps ?? config?.analysisFps ?? 0
  const actualStatus = primaryInstance?.actualStatus ?? config?.actualStatus ?? 0
  const runtimeStatus = getPipelineRuntimeStatus(actualStatus, t)
  // 配置收敛态势与运行状态正交：运行中也可能存在“期望配置未生效”的实例
  const applySummary = summarizeInstanceApply(algorithmInstances)

  const [copied, setCopied] = useState(false)
  const probeBadge = getProbeBadge(camera.lastProbeStatus, t)

  const handleCopyRtsp = async (e: React.MouseEvent) => {
    e.stopPropagation()
    const success = await copyToClipboard(camera.rtspUrl)
    if (success) {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }
  }

  return (
    <motion.article
      variants={{
        hidden: { opacity: 0, y: reduceMotion ? 0 : motionTokens.distance.md },
        visible: {
          opacity: 1,
          y: 0,
          transition: {
            duration: reduceMotion ? 0 : motionTokens.duration.normal,
            ease: motionTokens.easing.smooth,
          },
        },
      }}
      whileHover={reduceMotion ? undefined : { y: -2 }}
      transition={{ duration: 0.2, ease: motionTokens.easing.smooth }}
      className={`group relative flex flex-col overflow-hidden rounded-2xl border p-4 text-left backdrop-blur-xl transition-all duration-300 ${
        isArmed
          ? 'border-emerald-500/40 bg-white/80 shadow-sm ring-1 ring-emerald-500/20 hover:border-emerald-500/60 hover:shadow-[0_8px_30px_rgba(16,185,129,0.12)] dark:bg-[#0b0e14]/80'
          : 'border-black/[0.07] bg-white/70 shadow-sm hover:border-black/15 hover:shadow-lg dark:border-white/[0.08] dark:bg-[#0b0e14]/65 dark:hover:border-white/15'
      }`}
      style={{
        boxShadow: isArmed
          ? 'inset 0 1px 0 0 rgba(255, 255, 255, 0.12), 0 4px 20px rgba(0, 0, 0, 0.04)'
          : 'inset 0 1px 0 0 rgba(255, 255, 255, 0.08)',
      }}
    >
      {/* 1. 头部：身份 + 操作 */}
      <div className="relative z-10 flex items-start justify-between gap-2">
        <div className="flex min-w-0 items-center gap-3">
          <div className="relative flex h-10 w-10 shrink-0 items-center justify-center rounded-xl border border-black/5 bg-gradient-to-br from-blue-500/10 to-indigo-500/10 shadow-2xs dark:border-white/10">
            <Video className="h-4 w-4 text-[var(--accent)]" />
            <span
              className={`absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-white dark:border-[#0b0e14] ${probeBadge.dotClass}`}
            />
          </div>
          <div className="min-w-0">
            <h4 className="truncate text-sm font-bold tracking-tight text-[var(--text-primary)]">
              {camera.name || camera.cameraId}
            </h4>
            <p className="truncate font-mono text-[11px] text-[var(--text-muted)]">
              {camera.cameraId}
            </p>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          {/* 布防总闸 */}
          <motion.button
            type="button"
            onClick={onToggleArm}
            whileHover={reduceMotion ? undefined : { scale: 1.02 }}
            whileTap={reduceMotion ? undefined : { scale: 0.95 }}
            aria-pressed={isArmed}
            aria-label={isArmed ? t('status.armed') : t('status.disarmed')}
            title={isArmed ? t('status.armed') : t('status.disarmed')}
            className={`flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-semibold transition-all ${
              isArmed
                ? 'border-emerald-500/35 bg-emerald-500/15 text-emerald-500 shadow-[0_0_12px_rgba(16,185,129,0.25)] hover:bg-emerald-500/20'
                : 'border-black/10 bg-black/5 text-[var(--text-muted)] hover:border-black/20 hover:text-[var(--text-primary)] dark:border-white/10 dark:bg-white/5 dark:hover:border-white/20'
            }`}
          >
            <ShieldCheck className="h-3.5 w-3.5" />
            <span className="hidden sm:inline">
              {isArmed ? t('status.armed') : t('status.disarmed')}
            </span>
          </motion.button>

          <motion.button
            type="button"
            onClick={onConfigure}
            whileHover={reduceMotion ? undefined : { scale: 1.05 }}
            whileTap={reduceMotion ? undefined : { scale: 0.92 }}
            title={t('actions.configureRules', { defaultValue: '配置算法与布防规则' })}
            aria-label={t('actions.configureRules', { defaultValue: '配置算法与布防规则' })}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-black/5 bg-black/5 text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:bg-[var(--accent)]/10 hover:text-[var(--accent)] dark:border-white/10 dark:bg-white/5"
          >
            <Pencil className="h-3.5 w-3.5" />
          </motion.button>

          <motion.button
            type="button"
            onClick={onDelete}
            whileHover={reduceMotion ? undefined : { scale: 1.05 }}
            whileTap={reduceMotion ? undefined : { scale: 0.92 }}
            title={t('deleteTask', { defaultValue: '删除布防任务' })}
            aria-label={t('deleteTask', { defaultValue: '删除布防任务' })}
            className="flex h-7 w-7 items-center justify-center rounded-lg border border-black/5 bg-black/5 text-[var(--text-secondary)] transition-colors hover:border-[var(--destructive)]/40 hover:bg-[var(--destructive)]/10 hover:text-[var(--destructive)] dark:border-white/10 dark:bg-white/5"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </motion.button>
        </div>
      </div>

      {/* 2. 防区预览（语义化入口） */}
      <motion.button
        type="button"
        onClick={onConfigure}
        whileHover={reduceMotion ? undefined : { scale: 1.005 }}
        whileTap={reduceMotion ? undefined : { scale: 0.99 }}
        aria-label={`${t('actions.configureRules', { defaultValue: '配置算法与布防规则' })} - ${camera.name || camera.cameraId}`}
        className="group/canvas relative mt-3.5 aspect-video w-full overflow-hidden rounded-xl border border-black/10 bg-[#05070c] shadow-inner transition-all hover:border-[var(--accent)]/50 hover:shadow-[0_0_24px_rgba(59,130,246,0.18)] dark:border-white/10"
      >
        {/* 背景微米点阵 + 雷达网格 */}
        <span
          className="pointer-events-none absolute inset-0 opacity-25"
          style={{
            backgroundImage:
              'radial-gradient(circle, rgba(255,255,255,0.22) 1px, transparent 1px), linear-gradient(to right, rgba(255,255,255,0.04) 1px, transparent 1px), linear-gradient(to bottom, rgba(255,255,255,0.04) 1px, transparent 1px)',
            backgroundSize: '16px 16px',
          }}
        />
        {/* 四角高精工业 HUD 取景标 */}
        <span className="pointer-events-none absolute top-1.5 left-1.5 h-2 w-2 border-t-2 border-l-2 border-[var(--accent)]/60" />
        <span className="pointer-events-none absolute top-1.5 right-1.5 h-2 w-2 border-t-2 border-r-2 border-[var(--accent)]/60" />
        <span className="pointer-events-none absolute bottom-1.5 left-1.5 h-2 w-2 border-b-2 border-l-2 border-[var(--accent)]/60" />
        <span className="pointer-events-none absolute right-1.5 bottom-1.5 h-2 w-2 border-r-2 border-b-2 border-[var(--accent)]/60" />

        <svg
          className="pointer-events-none absolute inset-0 h-full w-full"
          viewBox="0 0 100 100"
          preserveAspectRatio="none"
        >
          <defs>
            <pattern
              id={`card-mask-hatch-${camera.cameraId}`}
              width="6"
              height="6"
              patternTransform="rotate(45 0 0)"
              patternUnits="userSpaceOnUse"
            >
              <line
                x1="0"
                y1="0"
                x2="0"
                y2="6"
                stroke="rgba(244, 63, 94, 0.45)"
                strokeWidth="1.5"
              />
            </pattern>
          </defs>

          {/* 极轻微的雷达十字与同心标尺 */}
          <circle
            cx="50"
            cy="50"
            r="35"
            fill="none"
            stroke="rgba(255,255,255,0.05)"
            strokeDasharray="2 3"
          />
          <line
            x1="50"
            y1="0"
            x2="50"
            y2="100"
            stroke="rgba(255,255,255,0.04)"
            strokeDasharray="1 3"
          />
          <line
            x1="0"
            y1="50"
            x2="100"
            y2="50"
            stroke="rgba(255,255,255,0.04)"
            strokeDasharray="1 3"
          />

          {rulesCount > 0 &&
            (() => {
              let roiIdx = 0
              return rules.map((rule, idx) => {
                const currentRoi = rule.role === 'roi' ? roiIdx++ : 0
                return renderPreviewRuleShape(rule, idx, camera.cameraId, currentRoi)
              })
            })()}
        </svg>

        {rulesCount === 0 && (
          <span className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1.5 text-[var(--text-muted)]">
            <Hexagon className="h-6 w-6 text-[var(--accent)] opacity-40 transition-transform group-hover/canvas:scale-110" />
            <span className="font-mono text-[11px] tracking-wide text-[var(--text-secondary)] opacity-85">
              {t('card.noRulesPlaceholder')}
            </span>
          </span>
        )}

        {/* 左上：规格与编码 */}
        <span className="pointer-events-none absolute top-2 left-2 z-10 flex flex-wrap items-center gap-1.5">
          <span className="flex items-center gap-1.5 rounded-md border border-white/10 bg-black/80 px-2 py-0.5 font-mono text-[10px] text-white/90 shadow-sm backdrop-blur-md">
            <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent)] shadow-[0_0_6px_var(--accent)]" />
            <span className="font-semibold text-[var(--accent)]">
              {camera.lastCodec?.toUpperCase() || 'H264'}
            </span>
            <span className="opacity-30">/</span>
            <span className="text-white/80">
              {camera.lastWidth ? `${camera.lastWidth}×${camera.lastHeight}` : '1080P'}
            </span>
          </span>
          {rulesCount > 0 && (
            <span className="flex items-center gap-1 font-mono text-[10px]">
              {roiCount > 0 && (
                <span className="rounded-md border border-[var(--accent)]/30 bg-[var(--accent)]/15 px-1.5 py-0.5 font-semibold text-[var(--accent)] backdrop-blur-xs">
                  {roiCount} ROI
                </span>
              )}
              {lineCount > 0 && (
                <span className="rounded-md border border-[var(--accent-green)]/30 bg-[var(--accent-green)]/15 px-1.5 py-0.5 font-semibold text-[var(--accent-green)] backdrop-blur-xs">
                  {lineCount} LINE
                </span>
              )}
              {maskCount > 0 && (
                <span className="rounded-md border border-[var(--destructive)]/30 bg-[var(--destructive)]/15 px-1.5 py-0.5 font-semibold text-[var(--destructive)] backdrop-blur-xs">
                  {maskCount} MASK
                </span>
              )}
              {precropCount > 0 && (
                <span className="rounded-md border border-lime-400/30 bg-lime-500/15 px-1.5 py-0.5 font-semibold text-lime-400 backdrop-blur-xs">
                  CROP
                </span>
              )}
            </span>
          )}
        </span>

        {/* 右上：进入工作台提示 */}
        <span className="absolute top-2 right-2 z-10 flex items-center gap-1 rounded-lg border border-white/20 bg-white/15 px-2.5 py-1 text-[11px] font-semibold text-white opacity-0 shadow-lg backdrop-blur-md transition-all duration-200 group-hover:opacity-100">
          <Pencil className="h-3 w-3" />
          <span>{t('card.enterRules', { defaultValue: '配置算法与规则' })}</span>
        </span>

        {/* 悬停底部遮罩提示 */}
        <span className="absolute inset-x-0 bottom-0 z-10 flex items-center justify-center gap-1.5 border-t border-white/10 bg-black/75 py-2 font-mono text-[11px] font-medium text-white/90 opacity-0 backdrop-blur-md transition-opacity duration-200 group-hover:opacity-100">
          <Layers className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>{t('card.enterStudioHint', { defaultValue: '进入布防工作台' })} ↗</span>
        </span>
      </motion.button>

      {/* 3. 配置摘要微磁贴（Frosted Micro-tiles） */}
      <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
        <div className="flex min-w-0 items-center justify-between gap-2 rounded-xl border border-black/[0.06] bg-black/[0.02] px-3 py-2 backdrop-blur-md dark:border-white/[0.08] dark:bg-white/[0.025]">
          <span className="flex min-w-0 items-center gap-1.5 text-[var(--text-secondary)]">
            <Hexagon className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
            <span className="truncate whitespace-nowrap">{t('card.geometryRules')}</span>
          </span>
          <span className="shrink-0 font-semibold whitespace-nowrap text-[var(--accent)]">
            {t('card.rulesCount', { count: rulesCount })}
          </span>
        </div>

        <div className="flex min-w-0 items-center justify-between gap-2 rounded-xl border border-black/[0.06] bg-black/[0.02] px-3 py-2 backdrop-blur-md dark:border-white/[0.08] dark:bg-white/[0.025]">
          <span className="flex min-w-0 items-center gap-1.5 text-[var(--text-secondary)]">
            <Activity className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
            <span className="truncate whitespace-nowrap">{t('card.motionGate')}</span>
          </span>
          <span
            className={`shrink-0 font-semibold whitespace-nowrap ${isMotionGateEco ? 'text-emerald-500' : 'text-[var(--accent-amber)]'}`}
          >
            {isMotionGateEco ? t('card.motionGateEco') : t('card.motionGateAlways')}
          </span>
        </div>
      </div>

      {/* 4. 算法绑定与运行状态 */}
      <div className="mt-2 flex items-center justify-between gap-2 rounded-xl border border-black/[0.06] bg-black/[0.02] px-3 py-2 text-xs backdrop-blur-md dark:border-white/[0.08] dark:bg-white/[0.025]">
        <span className="flex min-w-0 items-center gap-1.5">
          <Layers className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
          {algorithmId ? (
            <span className="flex min-w-0 items-center gap-1.5">
              <span className="truncate font-mono font-semibold text-[var(--text-primary)]">
                {algorithmId}
              </span>
              {enabledInstances.length > 1 && (
                <span className="shrink-0 rounded-md border border-black/10 bg-black/5 px-1.5 font-mono text-[10px] text-[var(--text-muted)] dark:border-white/10 dark:bg-white/5">
                  +{enabledInstances.length - 1}
                </span>
              )}
            </span>
          ) : (
            <span className="truncate text-[var(--text-muted)]">
              {t('card.algorithmUnbound', { defaultValue: '未绑定算法' })}
            </span>
          )}
        </span>
        <span className="flex shrink-0 items-center gap-2 font-mono text-[11px]">
          <span className="rounded-md border border-black/5 bg-black/5 px-1.5 py-0.5 text-[10px] text-[var(--text-muted)] dark:border-white/10 dark:bg-white/5">
            {analysisFps > 0 ? `${analysisFps} FPS` : t('card.fpsAuto', { defaultValue: '自动' })}
          </span>
          <span className={`flex items-center gap-1 font-semibold ${runtimeStatus.className}`}>
            <span className="h-1.5 w-1.5 rounded-full bg-current" />
            <span>{runtimeStatus.label}</span>
          </span>
          {applySummary.tone !== 'ok' && (
            <span
              className={`inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[10px] font-semibold ${
                applySummary.tone === 'failed'
                  ? 'border-[var(--destructive)]/40 bg-[var(--destructive)]/10 text-[var(--destructive)]'
                  : 'border-[var(--accent-amber)]/40 bg-[var(--accent-amber)]/10 text-[var(--accent-amber)]'
              }`}
              title={unappliedNoticeLines(applySummary).join('\n')}
            >
              <TriangleAlert className="h-2.5 w-2.5 shrink-0" />
              <span>
                {applySummary.tone === 'failed'
                  ? t('card.applyFailed', { defaultValue: '配置未生效' })
                  : t('card.applyPending', { defaultValue: '配置排队中' })}
              </span>
            </span>
          )}
        </span>
      </div>

      {/* 5. 接入状态与码流/地址 */}
      <div className="mt-2.5 flex items-center justify-between gap-2 border-t border-black/5 pt-3 dark:border-white/10">
        <span
          className={`inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11px] font-medium ${probeBadge.badgeBg}`}
        >
          <span className={`h-1.5 w-1.5 rounded-full ${probeBadge.dotClass}`} />
          <span>{probeBadge.text}</span>
        </span>

        <div className="flex min-w-0 items-center gap-1.5">
          {/* 码流快捷切换 */}
          {onStreamModeChange &&
            (() => {
              const streamConfig = STREAM_MODE_CONFIG[camera.streamMode || 'auto']
              return (
                <button
                  type="button"
                  onClick={() => onStreamModeChange(nextStreamMode(camera.streamMode))}
                  title={t('card.clickToSwitchStreamMode', {
                    defaultValue: '点击可快捷切换分析码流 (主码流 / 子码流 / 自动)',
                  })}
                  className={`flex shrink-0 items-center gap-1 rounded-lg border px-2 py-0.5 font-mono text-[10px] font-semibold transition-all ${streamConfig.className}`}
                >
                  <span>
                    {t(streamConfig.labelKey, { defaultValue: streamConfig.defaultLabel })}
                  </span>
                  <ArrowLeftRight className="h-2.5 w-2.5 opacity-70" />
                </button>
              )
            })()}

          {/* RTSP 地址复制 */}
          <button
            type="button"
            onClick={handleCopyRtsp}
            title={`${t('card.copyRtsp')}\n${camera.rtspUrl}`}
            aria-label={`${t('card.copyRtsp')}: ${camera.rtspUrl}`}
            className="flex shrink-0 items-center gap-1 rounded-lg border border-black/5 bg-black/5 px-2 py-0.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent)]/10 hover:text-[var(--accent)] dark:border-white/10 dark:bg-white/5"
          >
            <Radio className="h-3.5 w-3.5" />
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
      </div>
    </motion.article>
  )
}
