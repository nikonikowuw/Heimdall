import React, { useRef, useState } from 'react'
import {
  AlertCircle,
  CheckCircle2,
  FileArchive,
  Loader2,
  ShieldCheck,
  Upload,
  X,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { algorithmApi } from '@/lib/api'
import { motionTokens } from '@/lib/motionTokens'
import type { UploadAlgorithmResponse } from '@/types'

export interface UploadModalProps {
  isOpen: boolean
  onClose: () => void
  onSuccess: () => void
}

export const UploadModal: React.FC<UploadModalProps> = ({ isOpen, onClose, onSuccess }) => {
  const { t } = useTranslation('algo')
  const fileInputRef = useRef<HTMLInputElement>(null)

  const stepsList = [
    t('upload.step1'),
    t('upload.step2'),
    t('upload.step3'),
    t('upload.step4'),
    t('upload.step5'),
    t('upload.step6'),
    t('upload.step7'),
  ]

  const [selectedFile, setSelectedFile] = useState<File | null>(null)
  const [isUploading, setIsUploading] = useState(false)
  const [uploadResult, setUploadResult] = useState<UploadAlgorithmResponse | null>(null)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [currentStepIdx, setCurrentStepIdx] = useState(0)

  if (!isOpen) return null

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (e.target.files && e.target.files[0]) {
      setSelectedFile(e.target.files[0])
      setUploadResult(null)
      setErrorMsg(null)
      setCurrentStepIdx(0)
    }
  }

  const handleDrop = (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault()
    if (e.dataTransfer.files && e.dataTransfer.files[0]) {
      setSelectedFile(e.dataTransfer.files[0])
      setUploadResult(null)
      setErrorMsg(null)
      setCurrentStepIdx(0)
    }
  }

  const handleUploadAndVerify = async () => {
    if (!selectedFile) return

    setIsUploading(true)
    setErrorMsg(null)
    setCurrentStepIdx(1)

    // 动态步进动画定时器模拟物理沙箱前置校验推进
    const timer = setInterval(() => {
      setCurrentStepIdx((prev) => (prev < 6 ? prev + 1 : prev))
    }, 600)

    try {
      const res = await algorithmApi.uploadPackage(selectedFile)
      clearInterval(timer)
      setUploadResult(res)
      if (res.passed) {
        setCurrentStepIdx(7)
        onSuccess()
      } else {
        setCurrentStepIdx(res.stepsPassed)
        setErrorMsg(res.errorMessage || t('upload.failedTitle'))
      }
    } catch (e: unknown) {
      clearInterval(timer)
      const msg = e instanceof Error ? e.message : t('upload.failedTitle')
      setErrorMsg(msg)
    } finally {
      setIsUploading(false)
    }
  }

  const handleReset = () => {
    setSelectedFile(null)
    setUploadResult(null)
    setErrorMsg(null)
    setCurrentStepIdx(0)
  }

  return (
    <AnimatePresence>
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
        {/* 背景遮罩 */}
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: motionTokens.duration.fast }}
          onClick={onClose}
          className="fixed inset-0 bg-black/60 backdrop-blur-xs"
        />

        {/* 弹窗主体 */}
        <motion.div
          initial={{ opacity: 0, scale: 0.96 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.96 }}
          transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
          className="frosted-glass relative z-10 w-full max-w-lg rounded-3xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-6 shadow-2xl"
        >
          {/* 标题栏 */}
          <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
            <div className="flex items-center gap-2.5">
              <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
                <Upload className="h-5 w-5" />
              </div>
              <div>
                <h3 className="text-sm font-bold text-[var(--text-primary)]">
                  {t('upload.title')}
                </h3>
                <p className="text-[11px] text-[var(--text-muted)]">{t('upload.subtitle')}</p>
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

          <div className="mt-4 space-y-4">
            {/* 未上传时：文件拖拽选择区 */}
            {!isUploading && !uploadResult && (
              <div
                onDragOver={(e) => e.preventDefault()}
                onDrop={handleDrop}
                onClick={() => fileInputRef.current?.click()}
                className="flex cursor-pointer flex-col items-center justify-center rounded-2xl border-2 border-dashed border-[var(--border)] bg-[var(--bg-secondary)] p-8 text-center transition-all hover:border-[var(--accent)] hover:bg-[var(--accent-soft)]/10"
              >
                <input
                  ref={fileInputRef}
                  type="file"
                  accept=".tar.gz,.tgz,.tar,.zip"
                  onChange={handleFileSelect}
                  className="hidden"
                />
                <FileArchive className="h-10 w-10 text-[var(--accent)]" />
                <span className="mt-3 text-xs font-semibold text-[var(--text-primary)]">
                  {selectedFile ? selectedFile.name : t('upload.dragTip')}
                </span>
                <span className="mt-1 text-[11px] text-[var(--text-muted)]">
                  {selectedFile
                    ? `${(selectedFile.size / 1024 / 1024).toFixed(2)} MB`
                    : t('upload.fileHint')}
                </span>
              </div>
            )}

            {/* 校验与流转展示区 */}
            {(isUploading || uploadResult || currentStepIdx > 0) && (
              <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4">
                <div className="mb-3 flex items-center justify-between text-xs font-semibold">
                  <span className="text-[var(--text-primary)]">
                    {uploadResult?.passed
                      ? t('upload.passedTitle')
                      : errorMsg
                        ? t('upload.failedTitle')
                        : t('upload.testingTitle')}
                  </span>
                  <span className="font-mono text-[var(--accent)]">
                    {Math.min(currentStepIdx, 7)} / 7
                  </span>
                </div>

                {/* 七步自检列表 */}
                <div className="space-y-2">
                  {stepsList.map((step, idx) => {
                    const stepNum = idx + 1
                    const isFinished =
                      currentStepIdx >= stepNum && (!errorMsg || currentStepIdx > stepNum)
                    const isFailed = errorMsg && currentStepIdx === stepNum
                    const isRunning = isUploading && currentStepIdx === stepNum

                    return (
                      <div
                        key={step}
                        className={`flex items-center justify-between rounded-xl px-3 py-2 text-xs transition-all ${
                          isFinished
                            ? 'bg-emerald-500/10 text-emerald-400'
                            : isFailed
                              ? 'bg-red-500/10 text-red-400'
                              : isRunning
                                ? 'bg-[var(--accent-soft)] text-[var(--accent)]'
                                : 'text-[var(--text-muted)] opacity-50'
                        }`}
                      >
                        <span className="font-medium">{step}</span>
                        {isFinished && <CheckCircle2 className="h-4 w-4 text-emerald-400" />}
                        {isFailed && <AlertCircle className="h-4 w-4 text-red-400" />}
                        {isRunning && (
                          <Loader2 className="h-4 w-4 animate-spin text-[var(--accent)]" />
                        )}
                      </div>
                    )
                  })}
                </div>
              </div>
            )}

            {/* 错误提示框 */}
            {errorMsg && (
              <div className="flex items-start gap-2.5 rounded-xl border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-400">
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
                <div className="space-y-1">
                  <span className="font-semibold">{t('upload.failedTitle')}</span>
                  <p className="leading-relaxed opacity-90">{errorMsg}</p>
                </div>
              </div>
            )}

            {/* 成功落库提示框 */}
            {uploadResult?.passed && uploadResult.version && (
              <div className="flex items-center gap-3 rounded-xl border border-emerald-500/20 bg-emerald-500/10 p-3.5 text-xs text-emerald-400">
                <ShieldCheck className="h-5 w-5 shrink-0" />
                <div>
                  <span className="font-bold">{t('upload.uploadSuccess')}</span>
                  <p className="mt-0.5 font-mono text-[11px] opacity-80">
                    {uploadResult.version.algorithmId} v{uploadResult.version.version} (
                    {uploadResult.version.platformId})
                  </p>
                </div>
              </div>
            )}
          </div>

          {/* 底部按钮栏 */}
          <div className="mt-6 flex items-center justify-end gap-2.5 border-t border-[var(--border)] pt-4">
            <button
              type="button"
              onClick={onClose}
              className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
            >
              {t('actions.close')}
            </button>

            {errorMsg && (
              <button
                type="button"
                onClick={handleReset}
                className="rounded-xl bg-[var(--bg-secondary)] px-4 py-2 text-xs font-medium text-[var(--text-primary)] transition-colors hover:bg-[var(--border)]"
              >
                {t('upload.reupload')}
              </button>
            )}

            {!uploadResult?.passed && !errorMsg && (
              <button
                type="button"
                disabled={!selectedFile || isUploading}
                onClick={handleUploadAndVerify}
                className="flex items-center gap-2 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-semibold text-white shadow-md transition-all hover:opacity-90 disabled:opacity-50"
              >
                {isUploading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                <span>{t('actions.uploadPackage')}</span>
              </button>
            )}
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  )
}
