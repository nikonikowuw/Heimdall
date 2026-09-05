import React, { useState } from 'react'
import {
  AlertCircle,
  CheckCircle2,
  HardDrive,
  Layers,
  Radio,
  ShieldCheck,
  Trash2,
  X,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { algorithmApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmItem, AlgorithmVersionItem } from '@/types'

export interface VersionsDrawerProps {
  isOpen: boolean
  algorithm: AlgorithmItem | null
  onClose: () => void
  onRefresh: () => void
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`
}

export const VersionsDrawer: React.FC<VersionsDrawerProps> = ({
  isOpen,
  algorithm,
  onClose,
  onRefresh,
}) => {
  const { t } = useTranslation('algo')
  const [operatingVersion, setOperatingVersion] = useState<string | null>(null)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [versionToUninstall, setVersionToUninstall] = useState<AlgorithmVersionItem | null>(null)

  if (!isOpen || !algorithm) return null

  const handleActivate = async (version: string) => {
    setOperatingVersion(version)
    setErrorMsg(null)
    try {
      await algorithmApi.activateVersion(algorithm.algorithmId, version)
      onRefresh()
    } catch (e: unknown) {
      setErrorMsg(
        e instanceof Error
          ? e.message
          : t('drawer.activateFailed', { defaultValue: '激活版本失败' }),
      )
    } finally {
      setOperatingVersion(null)
    }
  }

  const handleUninstall = async (versionItem: AlgorithmVersionItem) => {
    setOperatingVersion(versionItem.version)
    setErrorMsg(null)
    try {
      await algorithmApi.uninstallVersion(algorithm.algorithmId, versionItem.version)
      setVersionToUninstall(null)
      onRefresh()
    } catch (e: unknown) {
      setErrorMsg(
        e instanceof Error
          ? e.message
          : t('drawer.uninstallFailed', { defaultValue: '卸载版本失败' }),
      )
    } finally {
      setOperatingVersion(null)
    }
  }

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex justify-end">
        {/* 背景遮罩 */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: motionTokens.duration.fast }}
          onClick={onClose}
          className="fixed inset-0 bg-black/50 backdrop-blur-xs"
        />

        {/* 抽屉面板 */}
        <motion.div
          initial={{ x: '100%' }}
          animate={{ x: 0 }}
          exit={{ x: '100%' }}
          transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
          className="frosted-glass relative z-10 flex h-full w-full max-w-md flex-col border-l border-[var(--border)] bg-[var(--bg-surface-solid)] p-6 shadow-2xl"
        >
          {/* 抽屉头部 */}
          <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
            <div className="flex items-center gap-2.5">
              <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
                <Layers className="h-5 w-5" />
              </div>
              <div>
                <h3 className="text-sm font-bold text-[var(--text-primary)]">{algorithm.name}</h3>
                <span className="font-mono text-xs text-[var(--accent)]">
                  {algorithm.algorithmId}
                </span>
              </div>
            </div>
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
            >
              <X className="h-4 w-4" />
            </button>
          </div>

          {/* 错误提示栏 */}
          {errorMsg && (
            <div className="mt-4 flex items-center gap-2 rounded-xl border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-400">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{errorMsg}</span>
            </div>
          )}

          {/* 版本列表 */}
          <div className="mt-4 flex-1 space-y-3 overflow-y-auto pr-1">
            <div className="flex items-center justify-between text-xs font-semibold text-[var(--text-muted)]">
              <span>{t('drawer.versionList')}</span>
              <span>{t('drawer.versionCount', { count: algorithm.versions.length })}</span>
            </div>

            {algorithm.versions.map((v) => {
              const isActive = v.isActive || v.version === algorithm.activeVersion
              const isOperating = operatingVersion === v.version

              return (
                <div
                  key={`${v.algorithmId}-${v.version}-${v.platformId}`}
                  className={`relative rounded-xl border p-4 transition-all ${
                    isActive
                      ? 'border-[var(--accent)] bg-[var(--accent-soft)]/20'
                      : 'border-[var(--border)] bg-[var(--bg-secondary)]'
                  }`}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="space-y-1">
                      <div className="flex items-center gap-2">
                        <span className="font-mono text-sm font-bold text-[var(--text-primary)]">
                          v{v.version}
                        </span>
                        {isActive && (
                          <span className="inline-flex items-center gap-1 rounded-md bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold text-emerald-400">
                            <CheckCircle2 className="h-3 w-3" />
                            {t('actions.active')}
                          </span>
                        )}
                        {v.isBuiltin && (
                          <span className="inline-flex items-center gap-1 rounded-md bg-indigo-500/10 px-1.5 py-0.5 text-[10px] font-medium text-indigo-400">
                            <ShieldCheck className="h-3 w-3" />
                            {t('card.builtinTag')}
                          </span>
                        )}
                      </div>

                      <div className="flex flex-wrap items-center gap-2 pt-1 text-[11px] text-[var(--text-muted)]">
                        <span className="font-mono text-xs">{v.platformId}</span>
                        <span>•</span>
                        <span className="inline-flex items-center gap-1">
                          <HardDrive className="h-3 w-3" />
                          {formatBytes(v.packageSizeBytes)}
                        </span>
                      </div>
                    </div>

                    {/* 操作按键 */}
                    <div className="flex items-center gap-1.5">
                      {!isActive && (
                        <button
                          type="button"
                          disabled={isOperating}
                          onClick={() => handleActivate(v.version)}
                          className="flex items-center gap-1 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2.5 py-1 text-xs font-medium text-[var(--text-primary)] transition-all hover:bg-[var(--accent)] hover:text-white disabled:opacity-50"
                        >
                          <Radio className="h-3 w-3 text-[var(--accent)]" />
                          <span>{t('actions.activate')}</span>
                        </button>
                      )}

                      {!v.isBuiltin && (
                        <button
                          type="button"
                          disabled={isOperating}
                          onClick={() => setVersionToUninstall(v)}
                          className="rounded-lg p-1.5 text-[var(--text-muted)] transition-colors hover:bg-red-500/10 hover:text-red-400 disabled:opacity-50"
                          title={t('actions.uninstall')}
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      )}
                    </div>
                  </div>

                  {/* 帧率档位标签 */}
                  {v.fpsTiers && v.fpsTiers.length > 0 && (
                    <div className="mt-3 border-t border-[var(--border)]/60 pt-2">
                      <span className="text-[10px] font-medium text-[var(--text-muted)]">
                        {t('drawer.fpsTiers')}:
                      </span>
                      <div className="mt-1 flex flex-wrap gap-1">
                        {v.fpsTiers.map((tier) => (
                          <span
                            key={tier.fps}
                            className="rounded-md border border-[var(--border)] bg-[var(--bg-surface-solid)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--text-secondary)]"
                          >
                            {tier.fps} FPS ({tier.units} units)
                          </span>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>

          {/* 卸载确认弹框 */}
          {versionToUninstall && (
            <div className="mt-4 rounded-xl border border-red-500/20 bg-red-500/10 p-4">
              <h4 className="text-xs font-bold text-red-400">
                {t('drawer.confirmUninstallTitle')}
              </h4>
              <p className="mt-1 text-xs text-[var(--text-secondary)]">
                {t('drawer.confirmUninstallDesc', { version: versionToUninstall.version })}
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
                  onClick={() => handleUninstall(versionToUninstall)}
                  className="rounded-lg bg-red-500 px-3 py-1 text-xs font-semibold text-white transition-opacity hover:opacity-90"
                >
                  {t('actions.uninstall')}
                </button>
              </div>
            </div>
          )}
        </motion.div>
      </div>
    </AnimatePresence>
  )
}
