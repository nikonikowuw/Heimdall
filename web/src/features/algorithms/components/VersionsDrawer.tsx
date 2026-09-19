import React, { useState } from 'react'
import {
  AlertCircle,
  CheckCircle2,
  ExternalLink,
  HardDrive,
  Layers,
  Loader2,
  Radio,
  RefreshCw,
  ShieldCheck,
  Trash2,
  TriangleAlert,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { algorithmApi } from '@/lib/api'
import { formatTimestampShort } from '@/lib/time'
import type { AlgorithmItem, AlgorithmVersionItem } from '@/types'
import { blockingUsageEntries, type AlgoUsageEntry } from '../algoUsage'
import { formatBytes } from '../format'
import { Overlay } from './Overlay'

export interface VersionsDrawerProps {
  isOpen: boolean
  algorithm: AlgorithmItem | null
  /** 绑定了该算法的通道任务；用于在卸载前说明「谁在用」 */
  usage: AlgoUsageEntry[]
  /** 当前任务占用清单是否仍在请求中；未知期间保守锁定卸载 */
  usageLoading?: boolean
  /** 当前任务占用清单读取失败；未知期间保守锁定卸载 */
  usageError?: boolean
  onClose: () => void
  onRefresh: () => void
  /** 跳转到任务布防页定位到指定通道 */
  onNavigateToTask?: (cameraId: string) => void
}

/**
 * 版本管理抽屉。
 *
 * 每个版本行必须同时回答：**能否在本机加载**（平台归一化后的兼容性）、
 * **适配器门槛**（`minAdapterVersion` 决定能不能被运行时加载）、**何时上传**（排查时间线）、
 * **能否下掉**（内置写保护 / 启用中占用），而不是让用户点下去再收到一个失败。
 */
export function VersionsDrawer({
  isOpen,
  algorithm,
  usage,
  usageLoading = false,
  usageError = false,
  onClose,
  onRefresh,
  onNavigateToTask,
}: VersionsDrawerProps): React.ReactElement {
  const { t, i18n } = useTranslation('algo')
  const [operatingVersionId, setOperatingVersionId] = useState<number | null>(null)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [versionToUninstall, setVersionToUninstall] = useState<AlgorithmVersionItem | null>(null)

  useDismissStack(isOpen, onClose, { disabled: operatingVersionId !== null })
  useDismissStack(isOpen && Boolean(versionToUninstall), () => setVersionToUninstall(null), {
    priority: 10,
    disabled: operatingVersionId !== null,
  })

  const blockingUsage = blockingUsageEntries(usage)
  const isInUse = blockingUsage.length > 0
  const usageUnknown = usageLoading || usageError

  const handleActivate = async (version: AlgorithmVersionItem) => {
    if (!algorithm) return
    setOperatingVersionId(version.id)
    setErrorMsg(null)
    try {
      await algorithmApi.activateVersion(algorithm.algorithmId, version.version)
      onRefresh()
    } catch (error: unknown) {
      setErrorMsg(error instanceof Error ? error.message : t('drawer.activateFailed'))
    } finally {
      setOperatingVersionId(null)
    }
  }

  const handleUninstall = async (version: AlgorithmVersionItem) => {
    if (!algorithm) return
    setOperatingVersionId(version.id)
    setErrorMsg(null)
    try {
      await algorithmApi.uninstallVersion(
        algorithm.algorithmId,
        version.version,
        version.platformId,
      )
      setVersionToUninstall(null)
      onRefresh()
    } catch (error: unknown) {
      setErrorMsg(error instanceof Error ? error.message : t('drawer.uninstallFailed'))
    } finally {
      setOperatingVersionId(null)
    }
  }

  return (
    <Overlay
      isOpen={isOpen && Boolean(algorithm)}
      onClose={onClose}
      ariaLabel={t('drawer.title')}
      variant="drawer"
    >
      <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <Layers className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <h3 className="truncate text-sm font-bold text-[var(--text-primary)]">
              {algorithm?.name ?? ''}
            </h3>
            <span className="font-data block truncate text-[11px] text-[var(--accent)]">
              {algorithm?.algorithmId ?? ''}
            </span>
          </div>
        </div>
        <button
          type="button"
          data-autofocus
          onClick={onClose}
          aria-label={t('actions.close')}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {errorMsg && (
        <div
          role="alert"
          className="mt-4 flex items-start gap-2.5 rounded-xl border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-400"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <p className="leading-relaxed">{errorMsg}</p>
        </div>
      )}

      {usageLoading && (
        <div
          role="status"
          className="mt-4 flex items-center gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3 text-xs text-[var(--text-secondary)]"
        >
          <Loader2 className="h-4 w-4 shrink-0 animate-spin text-[var(--accent)] motion-reduce:animate-none" />
          <span>{t('drawer.usageChecking')}</span>
        </div>
      )}

      {usageError && (
        <div
          role="alert"
          className="mt-4 flex items-start gap-2.5 rounded-xl border border-amber-500/20 bg-amber-500/10 p-3 text-xs text-amber-400"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0 flex-1">
            <p>{t('drawer.usageUnavailable')}</p>
            <button
              type="button"
              onClick={onRefresh}
              className="mt-2 inline-flex items-center gap-1.5 font-medium text-[var(--accent)] hover:underline focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
            >
              <RefreshCw className="h-3 w-3" />
              <span>{t('drawer.retryUsage')}</span>
            </button>
          </div>
        </div>
      )}

      {/* 占用汇总：卸载保护的前置说明，点击可跳转任务页处理 */}
      {isInUse && (
        <div className="mt-4 rounded-xl border border-amber-500/20 bg-amber-500/10 p-3 text-xs text-amber-400">
          <div className="flex items-center gap-2 font-semibold">
            <Radio className="h-3.5 w-3.5" />
            <span>{t('drawer.inUseTitle', { count: blockingUsage.length })}</span>
          </div>
          <ul className="mt-2 space-y-1">
            {blockingUsage.map((entry) => (
              <li key={entry.cameraId} className="flex items-center justify-between gap-2">
                <span className="truncate text-[var(--text-secondary)]">
                  {entry.taskName} ({entry.cameraId})
                </span>
                {onNavigateToTask && (
                  <button
                    type="button"
                    onClick={() => onNavigateToTask(entry.cameraId)}
                    className="flex shrink-0 items-center gap-1 text-[11px] font-medium text-[var(--accent)] hover:underline"
                  >
                    <ExternalLink className="h-3 w-3" />
                    {t('drawer.gotoTask')}
                  </button>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="mt-4 flex-1 space-y-3 overflow-y-auto pr-1">
        <div className="flex items-center justify-between text-xs font-semibold text-[var(--text-muted)]">
          <span>{t('drawer.versionList')}</span>
          <span>{t('drawer.versionCount', { count: algorithm?.versions.length ?? 0 })}</span>
        </div>

        {algorithm?.versions.map((version) => {
          const isActive = version.isActive || version.version === algorithm.activeVersion
          const isOperating = operatingVersionId === version.id
          const isConfirming = versionToUninstall?.id === version.id
          const platformAlias =
            version.platformId !== version.normalizedPlatformId ? version.platformId : null
          let uninstallTitle = t('actions.uninstall')
          if (usageUnknown) uninstallTitle = t('drawer.usageUnavailable')
          if (isInUse) uninstallTitle = t('drawer.inUseError')
          if (version.isBuiltin) uninstallTitle = t('drawer.builtinProtected')

          return (
            <div
              key={`${version.algorithmId}-${version.version}-${version.platformId}`}
              className={`rounded-xl border p-4 transition-colors ${
                isActive
                  ? 'border-[var(--accent)] bg-[var(--accent-soft)]/20'
                  : 'border-[var(--border)] bg-[var(--bg-secondary)]'
              }`}
            >
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0 space-y-1.5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="font-data text-sm font-bold text-[var(--text-primary)] tabular-nums">
                      v{version.version}
                    </span>
                    {isActive && (
                      <span className="inline-flex items-center gap-1 rounded-md bg-emerald-500/10 px-2 py-0.5 text-[11px] font-semibold text-emerald-400">
                        <CheckCircle2 className="h-3 w-3" />
                        {t('actions.active')}
                      </span>
                    )}
                    {version.isBuiltin && (
                      <span className="inline-flex items-center gap-1 rounded-md bg-indigo-500/10 px-1.5 py-0.5 text-[11px] font-medium text-indigo-400">
                        <ShieldCheck className="h-3 w-3" />
                        {t('card.builtinTag')}
                      </span>
                    )}
                    <span
                      className={`inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium ${
                        version.compatibleWithHost
                          ? 'bg-emerald-500/10 text-emerald-400'
                          : 'bg-amber-500/10 text-amber-400'
                      }`}
                    >
                      {!version.compatibleWithHost && <TriangleAlert className="h-3 w-3" />}
                      {version.compatibleWithHost
                        ? t('drawer.compatibleHost')
                        : t('drawer.incompatibleHost')}
                    </span>
                  </div>

                  <div className="font-data flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-[var(--text-muted)] tabular-nums">
                    <span title={platformAlias ?? version.platformId}>
                      {version.normalizedPlatformId}
                      {platformAlias ? ` (${platformAlias})` : ''}
                    </span>
                    <span aria-hidden="true">·</span>
                    <span className="inline-flex items-center gap-1">
                      <HardDrive className="h-3 w-3" />
                      {formatBytes(version.packageSizeBytes)}
                    </span>
                    <span aria-hidden="true">·</span>
                    <span>
                      {t('drawer.adapterVersion', { version: version.minAdapterVersion })}
                    </span>
                    <span aria-hidden="true">·</span>
                    <span>{formatTimestampShort(version.updatedAt, i18n.language)}</span>
                  </div>
                </div>

                <div className="flex shrink-0 items-center gap-1.5">
                  {!isActive && (
                    <button
                      type="button"
                      disabled={isOperating || !version.compatibleWithHost}
                      onClick={() => handleActivate(version)}
                      title={
                        version.compatibleWithHost
                          ? t('actions.activate')
                          : t('drawer.activateBlocked')
                      }
                      className="flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2.5 py-1 text-xs font-medium text-[var(--text-primary)] transition-all hover:bg-[var(--accent)] hover:text-white focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:opacity-40"
                    >
                      {isOperating ? (
                        <Loader2 className="h-3 w-3 animate-spin text-[var(--accent)]" />
                      ) : (
                        <Radio className="h-3 w-3 text-[var(--accent)]" />
                      )}
                      <span>{t('actions.activate')}</span>
                    </button>
                  )}

                  {/* 内置版本渲染禁用态而不是直接隐藏：可见性一致，且说明为什么不可删除 */}
                  <button
                    type="button"
                    disabled={
                      version.isBuiltin || isInUse || usageUnknown || isOperating || isConfirming
                    }
                    onClick={() => setVersionToUninstall(version)}
                    aria-label={t('actions.uninstall')}
                    title={uninstallTitle}
                    className="flex h-7 w-7 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-red-500/10 hover:text-red-400 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden disabled:opacity-30 disabled:hover:bg-transparent disabled:hover:text-[var(--text-muted)]"
                  >
                    <Trash2 className="h-4 w-4" />
                  </button>
                </div>
              </div>

              {version.fpsTiers && version.fpsTiers.length > 0 && (
                <div className="mt-3 border-t border-[var(--border)]/60 pt-2">
                  <span className="text-[11px] font-medium text-[var(--text-muted)]">
                    {t('drawer.fpsTiers')}
                  </span>
                  <div className="mt-1 flex flex-wrap gap-1">
                    {version.fpsTiers.map((tier) => (
                      <span
                        key={tier.fps}
                        className="font-data rounded-md border border-[var(--border)] bg-[var(--bg-surface-solid)] px-1.5 py-0.5 text-[11px] text-[var(--text-secondary)] tabular-nums"
                      >
                        {t('drawer.fpsTierValue', { fps: tier.fps, units: tier.units })}
                      </span>
                    ))}
                  </div>
                </div>
              )}

              {/* 卸载确认就近展开在被操作的版本行下方，避免确认按钮与目标行脱节 */}
              {isConfirming && (
                <div className="mt-3 rounded-xl border border-red-500/20 bg-red-500/10 p-3">
                  <h4 className="text-xs font-bold text-red-400">
                    {t('drawer.confirmUninstallTitle')}
                  </h4>
                  <p className="mt-1 text-xs leading-relaxed text-[var(--text-secondary)]">
                    {t('drawer.confirmUninstallDesc', {
                      version: version.version,
                      platform: version.platformId,
                    })}
                  </p>
                  <div className="mt-3 flex justify-end gap-2">
                    <button
                      type="button"
                      onClick={() => setVersionToUninstall(null)}
                      className="rounded-lg px-2.5 py-1 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)]"
                    >
                      {t('actions.cancel')}
                    </button>
                    <button
                      type="button"
                      disabled={isOperating}
                      onClick={() => handleUninstall(version)}
                      className="flex items-center gap-1.5 rounded-lg bg-red-500 px-3 py-1 text-xs font-semibold text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                    >
                      {isOperating && <Loader2 className="h-3 w-3 animate-spin" />}
                      <span>{t('actions.uninstall')}</span>
                    </button>
                  </div>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </Overlay>
  )
}
