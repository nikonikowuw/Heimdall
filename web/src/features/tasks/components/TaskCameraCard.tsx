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
  Video,
} from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { getProbeBadge } from '@/features/cameras/cameraStatus'
import { copyToClipboard } from '@/lib/utils'
import type { Camera, DetectionRule, StreamMode, TaskConfigDto } from '@/types'

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
  const isMotionGateEco = config?.motionGate?.enabled ?? false
  const algorithmInstances = config?.algorithmInstances ?? []
  const enabledInstances = algorithmInstances.filter((instance) => instance.enabled)
  const primaryInstance = enabledInstances[0] ?? algorithmInstances[0]
  const algorithmId = primaryInstance?.algorithmId ?? config?.algorithmId ?? ''
  const analysisFps = primaryInstance?.analysisFps ?? config?.analysisFps ?? 0
  const actualStatus = primaryInstance?.actualStatus ?? config?.actualStatus ?? 0
  const runtimeStatus = getPipelineRuntimeStatus(actualStatus, t)

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
      whileHover={reduceMotion ? undefined : { y: -1 }}
      transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
      className={`group relative flex flex-col overflow-hidden rounded-[8px] border border-l-4 bg-[var(--bg-surface-solid)] p-3.5 text-left shadow-[var(--shadow-sm)] transition-[border-color,box-shadow,transform] duration-200 ${
        isArmed
          ? 'border-[var(--accent-green)]/50 border-l-[var(--accent-green)] shadow-md hover:border-[var(--accent-green)] hover:shadow-lg'
          : 'border-[var(--border)] border-l-[var(--border-strong)] hover:border-[var(--border-strong)] hover:shadow-md'
      }`}
    >
      {/* 1. 头部：身份 + 操作 */}
      <div className="relative z-10 flex items-start justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="relative flex h-9 w-9 shrink-0 items-center justify-center rounded-[7px] border border-[var(--border)] bg-[var(--bg-secondary)]">
            <Video className="h-4 w-4 text-[var(--accent)]" />
            <span
              className={`absolute -top-0.5 -right-0.5 h-2.5 w-2.5 rounded-full border-2 border-[var(--bg-surface-solid)] ${probeBadge.dotClass}`}
            />
          </div>
          <div className="min-w-0">
            <h4 className="truncate text-sm font-bold text-[var(--text-primary)]">
              {camera.name || camera.cameraId}
            </h4>
            <p className="truncate font-mono text-[11px] text-[var(--text-muted)]">
              {camera.cameraId}
            </p>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1.5">
          {/* 布防总闸 */}
          <button
            type="button"
            onClick={onToggleArm}
            aria-pressed={isArmed}
            aria-label={isArmed ? t('status.armed') : t('status.disarmed')}
            title={isArmed ? t('status.armed') : t('status.disarmed')}
            className={`flex items-center gap-1.5 rounded-[6px] border px-2.5 py-1 text-xs font-semibold transition-all ${
              isArmed
                ? 'border-[var(--accent-green)]/35 bg-[var(--accent-green)]/10 text-[var(--accent-green)] hover:bg-[var(--accent-green)]/15'
                : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)] hover:border-[var(--accent)]/40 hover:text-[var(--text-primary)]'
            }`}
          >
            {isArmed ? (
              <>
                <ShieldCheck className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">{t('status.armed')}</span>
              </>
            ) : (
              <>
                <ShieldCheck className="h-3.5 w-3.5" />
                <span className="hidden sm:inline">{t('status.disarmed')}</span>
              </>
            )}
          </button>

          <button
            type="button"
            onClick={onConfigure}
            title={t('actions.configureRules', { defaultValue: '配置算法与布防规则' })}
            aria-label={t('actions.configureRules', { defaultValue: '配置算法与布防规则' })}
            className="flex h-7 w-7 items-center justify-center rounded-[6px] border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] transition-colors hover:border-[var(--accent)] hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>

          <button
            type="button"
            onClick={onDelete}
            title={t('deleteTask', { defaultValue: '删除布防任务' })}
            aria-label={t('deleteTask', { defaultValue: '删除布防任务' })}
            className="flex h-7 w-7 items-center justify-center rounded-[6px] border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] transition-colors hover:border-[var(--destructive)]/40 hover:bg-[var(--destructive)]/10 hover:text-[var(--destructive)]"
          >
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>

      {/* 2. 防区预览（唯一的进入工作台入口，语义化 button 避免嵌套误触） */}
      <button
        type="button"
        onClick={onConfigure}
        aria-label={`${t('actions.configureRules', { defaultValue: '配置算法与布防规则' })} - ${camera.name || camera.cameraId}`}
        className="relative mt-3 aspect-video w-full overflow-hidden rounded-[6px] border border-[var(--border)] bg-[var(--video-surface)] shadow-inner"
      >
        <span
          className="pointer-events-none absolute inset-0 opacity-20"
          style={{
            backgroundImage:
              'radial-gradient(circle, rgba(255,255,255,0.2) 1px, transparent 1px), linear-gradient(to right, rgba(255,255,255,0.05) 1px, transparent 1px), linear-gradient(to bottom, rgba(255,255,255,0.05) 1px, transparent 1px)',
            backgroundSize: '16px 16px',
          }}
        />
        <span className="pointer-events-none absolute top-1.5 left-1.5 h-2 w-2 border-t-2 border-l-2 border-white/40" />
        <span className="pointer-events-none absolute top-1.5 right-1.5 h-2 w-2 border-t-2 border-r-2 border-white/40" />
        <span className="pointer-events-none absolute bottom-1.5 left-1.5 h-2 w-2 border-b-2 border-l-2 border-white/40" />
        <span className="pointer-events-none absolute right-1.5 bottom-1.5 h-2 w-2 border-r-2 border-b-2 border-white/40" />

        {rulesCount > 0 ? (
          <svg
            className="pointer-events-none absolute inset-0 h-full w-full"
            viewBox="0 0 100 100"
            preserveAspectRatio="none"
          >
            {(() => {
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
          <span className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1 text-[var(--text-muted)]">
            <Hexagon className="h-5 w-5 text-[var(--accent)] opacity-50" />
            <span className="font-mono text-[11px] tracking-wide text-[var(--text-secondary)] opacity-80">
              {t('card.noRulesPlaceholder')}
            </span>
          </span>
        )}

        {/* 左上：规格与编码 */}
        <span className="pointer-events-none absolute top-2 left-2 z-10 flex items-center gap-1.5">
          <span className="flex items-center gap-1 rounded bg-black/75 px-2 py-0.5 font-mono text-[11px] text-white/90">
            <span className="font-semibold text-[var(--accent)]">
              {camera.lastCodec?.toUpperCase() || 'H264'}
            </span>
            <span className="opacity-40">/</span>
            <span>{camera.lastWidth ? `${camera.lastWidth}×${camera.lastHeight}` : '1080P'}</span>
          </span>
          {rulesCount > 0 && (
            <span className="flex items-center gap-1 font-mono text-[10px]">
              {roiCount > 0 && (
                <span className="rounded bg-[var(--accent)]/20 px-1.5 py-0.5 font-semibold text-[var(--accent)]">
                  {roiCount} ROI
                </span>
              )}
              {lineCount > 0 && (
                <span className="rounded bg-[var(--accent-green)]/20 px-1.5 py-0.5 font-semibold text-[var(--accent-green)]">
                  {lineCount} LINE
                </span>
              )}
              {maskCount > 0 && (
                <span className="rounded bg-[var(--destructive)]/20 px-1.5 py-0.5 font-semibold text-[var(--destructive)]">
                  {maskCount} MASK
                </span>
              )}
            </span>
          )}
        </span>

        {/* 右上：进入工作台提示 */}
        <span className="absolute top-2 right-2 z-10 rounded-md bg-[var(--accent)]/90 px-2 py-0.5 text-[11px] font-medium text-white opacity-0 shadow-md backdrop-blur-xs transition-opacity group-hover:opacity-100">
          {t('card.enterRules', { defaultValue: '配置算法与规则' })}
        </span>

        {/* 悬停遮罩提示 */}
        <span className="absolute inset-x-0 bottom-0 z-10 flex items-center justify-center gap-1.5 bg-black/70 py-1.5 text-[11px] font-medium text-white opacity-0 backdrop-blur-xs transition-opacity group-hover:opacity-100">
          <Layers className="h-3.5 w-3.5" />
          {t('card.enterStudioHint', { defaultValue: '进入布防工作台' })}
        </span>
      </button>

      {/* 3. 配置摘要：规则 / 门控 / 算法 */}
      <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
        <div className="flex min-w-0 items-center justify-between gap-2 rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1.5">
          <span className="flex min-w-0 items-center gap-1.5 text-[var(--text-secondary)]">
            <Hexagon className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
            <span className="truncate whitespace-nowrap">{t('card.geometryRules')}</span>
          </span>
          <span className="shrink-0 font-semibold whitespace-nowrap text-[var(--accent)]">
            {t('card.rulesCount', { count: rulesCount })}
          </span>
        </div>

        <div className="flex min-w-0 items-center justify-between gap-2 rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-1.5">
          <span className="flex min-w-0 items-center gap-1.5 text-[var(--text-secondary)]">
            <Activity className="h-3.5 w-3.5 shrink-0 text-[var(--accent-green)]" />
            <span className="truncate whitespace-nowrap">{t('card.motionGate')}</span>
          </span>
          <span
            className={`shrink-0 font-semibold whitespace-nowrap ${isMotionGateEco ? 'text-[var(--accent-green)]' : 'text-[var(--accent-amber)]'}`}
          >
            {isMotionGateEco ? t('card.motionGateEco') : t('card.motionGateAlways')}
          </span>
        </div>
      </div>

      {/* 4. 算法绑定与运行状态 */}
      <div className="mt-2 flex items-center justify-between gap-2 rounded-[6px] border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-2 text-xs">
        <span className="flex min-w-0 items-center gap-1.5">
          <Layers className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
          {algorithmId ? (
            <span className="flex min-w-0 items-center gap-1.5">
              <span className="truncate font-mono font-semibold text-[var(--text-primary)]">
                {algorithmId}
              </span>
              {enabledInstances.length > 1 && (
                <span className="shrink-0 rounded border border-[var(--border)] px-1 font-mono text-[10px] text-[var(--text-muted)]">
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
          <span className="text-[var(--text-muted)]">
            {analysisFps > 0 ? `${analysisFps} FPS` : t('card.fpsAuto', { defaultValue: '自动' })}
          </span>
          <span className={runtimeStatus.className}>{runtimeStatus.label}</span>
        </span>
      </div>

      {/* 5. 接入状态与码流/地址 */}
      <div className="mt-2 flex items-center justify-between gap-2 border-t border-[var(--border)] pt-2.5">
        <span
          className={`inline-flex shrink-0 items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-[11px] font-medium ${probeBadge.badgeBg}`}
        >
          <span className={`h-1.5 w-1.5 rounded-full ${probeBadge.dotClass}`} />
          <span>{probeBadge.text}</span>
        </span>

        <div className="flex min-w-0 items-center gap-1">
          {/* 码流循环切换 */}
          {onStreamModeChange && (
            <button
              type="button"
              onClick={() => onStreamModeChange(nextStreamMode(camera.streamMode))}
              title={t('card.clickToSwitchStreamMode', {
                defaultValue: '点击可快捷切换分析码流 (主码流 / 子码流 / 自动)',
              })}
              className={`flex shrink-0 items-center gap-1 rounded-lg border px-1.5 py-0.5 font-mono text-[10px] font-semibold transition-all ${
                camera.streamMode === 'main'
                  ? 'border-[var(--border-strong)] bg-[var(--accent-soft)] text-[var(--accent)]'
                  : camera.streamMode === 'sub'
                    ? 'border-[var(--accent-amber)]/50 bg-[var(--accent-amber)]/10 text-[var(--accent-amber)]'
                    : 'border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-secondary)]'
              }`}
            >
              {camera.streamMode === 'main'
                ? t('cardStream.main', { defaultValue: '主码流' })
                : camera.streamMode === 'sub'
                  ? t('cardStream.sub', { defaultValue: '子码流' })
                  : t('cardStream.auto', { defaultValue: '自动' })}
              <ArrowLeftRight className="h-2.5 w-2.5 opacity-70" />
            </button>
          )}

          {/* RTSP 地址复制 */}
          <button
            type="button"
            onClick={handleCopyRtsp}
            title={`${t('card.copyRtsp')}\n${camera.rtspUrl}`}
            aria-label={`${t('card.copyRtsp')}: ${camera.rtspUrl}`}
            className="flex shrink-0 items-center gap-1 rounded-lg px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
          >
            <Radio className="h-3.5 w-3.5" />
            {copied ? (
              <>
                <Check className="h-3 w-3 text-[var(--accent-green)]" />
                <span className="text-[var(--accent-green)]">{t('card.copied')}</span>
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
