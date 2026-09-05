import React from 'react'
import { Cpu, ExternalLink, ShieldCheck, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgoManifest } from '@/types'

export interface AlgoSandboxDrawerProps {
  isOpen: boolean
  algo: AlgoManifest | null
  onClose: () => void
  onNavigateToAlgorithms?: () => void
}

export function AlgoSandboxDrawer({
  isOpen,
  algo,
  onClose,
  onNavigateToAlgorithms,
}: AlgoSandboxDrawerProps): React.ReactElement {
  const { t } = useTranslation('task')

  return (
    <AnimatePresence>
      {isOpen && algo && (
        <motion.div
          key="algo-drawer-backdrop"
          role="dialog"
          aria-modal="true"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: motionTokens.duration.fast, ease: motionTokens.easing.smooth }}
          onClick={onClose}
          className="fixed inset-0 z-50 flex cursor-pointer justify-end bg-black/60 backdrop-blur-xs"
        >
          <motion.div
            key="algo-drawer-panel"
            initial={{ x: '100%' }}
            animate={{ x: 0 }}
            exit={{ x: '100%' }}
            transition={{
              duration: motionTokens.duration.normal,
              ease: motionTokens.easing.smooth,
            }}
            onClick={(e) => e.stopPropagation()}
            className="flex h-full w-96 cursor-default flex-col space-y-4 overflow-y-auto border-l border-[var(--border)] bg-[var(--bg-surface-solid)] p-5 shadow-2xl"
          >
            <div className="flex items-center justify-between border-b border-[var(--border)] pb-3">
              <div className="flex items-center gap-2.5">
                <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent-soft)] text-[var(--accent)]">
                  <Cpu className="h-4 w-4" />
                </div>
                <span className="text-base font-bold text-[var(--text-primary)]">{algo.name}</span>
              </div>
              <button
                type="button"
                onClick={onClose}
                className="rounded-lg p-1 text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
                title={t('studio.closeDrawerHint', { defaultValue: '关闭 (Esc / 点击遮罩)' })}
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="space-y-2.5 text-xs text-[var(--text-secondary)]">
              <div className="flex justify-between">
                <span className="text-[var(--text-muted)]">{t('algoDrawer.id')}</span>
                <span className="font-mono font-bold text-[var(--accent)]">{algo.algorithmId}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--text-muted)]">{t('algoDrawer.version')}</span>
                <span>
                  v{algo.version} ({algo.author})
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--text-muted)]">{t('algoDrawer.platforms')}</span>
                <span className="font-mono text-xs font-bold text-emerald-500">
                  {algo.supportedPlatforms.join(', ')}
                </span>
              </div>
              <p className="pt-1 text-xs leading-relaxed text-[var(--text-muted)]">
                {algo.description}
              </p>
            </div>

            <div className="space-y-2 border-t border-[var(--border)] pt-3">
              <span className="text-xs font-semibold text-[var(--text-primary)]">
                {t('algoDrawer.classes')}
              </span>
              <div className="flex flex-wrap gap-1.5">
                {algo.classes.map((cls) => (
                  <span
                    key={cls}
                    className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-2.5 py-0.5 font-mono text-xs font-medium text-[var(--accent)]"
                  >
                    {cls}
                  </span>
                ))}
              </div>
            </div>

            {/* 一级算法仓库解耦提示与导航入口 */}
            <div className="mt-4 rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 text-xs">
              <div className="flex items-center gap-2 font-semibold text-[var(--text-primary)]">
                <ShieldCheck className="h-4 w-4 text-emerald-500" />
                <span>
                  {t('algoDrawer.managementTitle', { defaultValue: '算法资产与版本管理' })}
                </span>
              </div>
              <p className="mt-2 text-[11px] leading-relaxed text-[var(--text-muted)]">
                {t('algoDrawer.managementDesc', {
                  defaultValue:
                    '当前摄像头已绑定此算法模型进行常驻子码流推理。如需上传新算法包归档、热切换模型版本或卸载资产，请通过控制台左侧导航前往「算法仓库」。',
                })}
              </p>
              {onNavigateToAlgorithms && (
                <button
                  type="button"
                  onClick={() => {
                    onClose()
                    onNavigateToAlgorithms()
                  }}
                  className="mt-3 flex w-full items-center justify-center gap-1.5 rounded-xl bg-[var(--accent)] py-2 text-xs font-semibold text-white shadow-xs transition-opacity hover:opacity-90"
                >
                  <span>{t('algoDrawer.goToRepo', { defaultValue: '前往算法仓库' })}</span>
                  <ExternalLink className="h-3.5 w-3.5" />
                </button>
              )}
            </div>

            <div className="flex justify-end pt-4">
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl bg-[var(--accent)] px-5 py-2 text-xs font-semibold text-white shadow-xs hover:opacity-90"
              >
                {t('algoDrawer.done')}
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
