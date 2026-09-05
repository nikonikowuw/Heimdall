import React from 'react'
import { AlertCircle, CheckCircle2, Cpu, ShieldCheck, Upload, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgoManifest, SandboxCheckResult } from '@/types'

export interface AlgoSandboxDrawerProps {
  isOpen: boolean
  algo: AlgoManifest | null
  onClose: () => void
  onUploadPackageFile: (e: React.ChangeEvent<HTMLInputElement>) => void
  isUploadingPkg: boolean
  onRunSandboxTest: () => void
  isVerifyingSandbox: boolean
  sandboxResult: SandboxCheckResult | null
}

export function AlgoSandboxDrawer({
  isOpen,
  algo,
  onClose,
  onUploadPackageFile,
  isUploadingPkg,
  onRunSandboxTest,
  isVerifyingSandbox,
  sandboxResult,
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

            {/* 上传算法包归档 */}
            <div className="space-y-2 border-t border-[var(--border)] pt-3">
              <div className="flex items-center justify-between">
                <span className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
                  <Upload className="h-4 w-4 text-[var(--accent)]" />
                  <span>{t('actions.uploadPackage')}</span>
                </span>
              </div>
              <label className="flex cursor-pointer items-center justify-center gap-2 rounded-xl border border-dashed border-[var(--border)] bg-[var(--bg-secondary)] p-3 text-xs text-[var(--text-secondary)] transition-all hover:border-[var(--accent)] hover:text-[var(--accent)]">
                <input
                  type="file"
                  accept=".zip,.tar,.tar.gz,.tgz"
                  className="hidden"
                  onChange={onUploadPackageFile}
                  disabled={isUploadingPkg}
                />
                <Upload className="h-4 w-4" />
                <span>{isUploadingPkg ? t('actions.uploading') : t('actions.uploadPackage')}</span>
              </label>
            </div>

            {/* 七步沙箱安全自检互动区 */}
            <div className="space-y-3 border-t border-[var(--border)] pt-3">
              <div className="flex items-center justify-between">
                <span className="flex items-center gap-1 text-xs font-semibold text-[var(--text-primary)]">
                  <ShieldCheck className="h-4 w-4 text-emerald-500" />
                  <span>{t('sandbox.title')}</span>
                </span>
                <button
                  type="button"
                  onClick={onRunSandboxTest}
                  disabled={isVerifyingSandbox}
                  className="text-xs font-semibold text-[var(--accent)] hover:underline disabled:opacity-50"
                >
                  {isVerifyingSandbox ? t('actions.verifyingSandbox') : t('actions.executeSandbox')}
                </button>
              </div>

              {sandboxResult ? (
                <div className="space-y-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3 text-xs">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-medium">{t('sandbox.status')}</span>
                    <span
                      className={`rounded-full px-2.5 py-0.5 font-mono text-xs font-bold ${
                        sandboxResult.passed
                          ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500'
                          : 'border border-rose-500/30 bg-rose-500/15 text-rose-500'
                      }`}
                    >
                      {sandboxResult.passed
                        ? t('sandbox.allPassed')
                        : t('sandbox.interrupted', { passed: sandboxResult.stepsPassed })}
                    </span>
                  </div>

                  <div className="space-y-1.5 pt-1">
                    {sandboxResult.steps.map((step, idx) => {
                      const isStepOk = idx < sandboxResult.stepsPassed
                      return (
                        <div
                          key={idx}
                          className="flex items-center gap-2 text-xs text-[var(--text-muted)]"
                        >
                          {isStepOk ? (
                            <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                          ) : (
                            <AlertCircle className="h-3.5 w-3.5 shrink-0 text-rose-500" />
                          )}
                          <span
                            className={isStepOk ? 'text-[var(--text-secondary)]' : 'text-rose-400'}
                          >
                            {step}
                          </span>
                        </div>
                      )
                    })}
                  </div>

                  {sandboxResult.errorMessage && (
                    <p className="mt-1 border-t border-[var(--border)] pt-1 font-mono text-xs text-rose-500">
                      {sandboxResult.errorMessage}
                    </p>
                  )}
                </div>
              ) : (
                <p className="text-xs text-[var(--text-muted)]">{t('sandbox.tip')}</p>
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
