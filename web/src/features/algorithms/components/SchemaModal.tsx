import React, { useState } from 'react'
import { Code2, Copy, FileJson, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { motionTokens } from '@/lib/motionTokens'
import type { AlgorithmItem } from '@/types'

export interface SchemaModalProps {
  isOpen: boolean
  algorithm: AlgorithmItem | null
  onClose: () => void
}

interface SchemaProperty {
  key: string
  type: string
  defaultVal: string
  description: string
}

export const SchemaModal: React.FC<SchemaModalProps> = ({ isOpen, algorithm, onClose }) => {
  const { t } = useTranslation('algo')
  const [showRaw, setShowRaw] = useState(false)
  const [copied, setCopied] = useState(false)

  if (!isOpen || !algorithm) return null

  const activeVersion =
    algorithm.versions.find((v) => v.isActive) ||
    algorithm.versions.find((v) => v.version === algorithm.activeVersion) ||
    algorithm.versions[0]

  const schemaObj = (activeVersion?.configSchema as Record<string, unknown>) || {}
  const propertiesObj = (schemaObj.properties as Record<string, Record<string, unknown>>) || {}

  const properties: SchemaProperty[] = Object.entries(propertiesObj).map(([k, v]) => ({
    key: k,
    type: String(v.type || 'unknown'),
    defaultVal: v.default !== undefined ? JSON.stringify(v.default) : '-',
    description: String(v.description || '-'),
  }))

  const rawJsonString = JSON.stringify(schemaObj, null, 2)

  const handleCopy = () => {
    navigator.clipboard.writeText(rawJsonString)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
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

        {/* 模态框主体 */}
        <motion.div
          initial={{ opacity: 0, scale: 0.96 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={{ opacity: 0, scale: 0.96 }}
          transition={{ duration: motionTokens.duration.normal, ease: motionTokens.easing.smooth }}
          className="frosted-glass relative z-10 w-full max-w-2xl rounded-3xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-6 shadow-2xl"
        >
          {/* 标题栏 */}
          <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
            <div className="flex items-center gap-2.5">
              <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
                <Code2 className="h-5 w-5" />
              </div>
              <div>
                <h3 className="text-sm font-bold text-[var(--text-primary)]">
                  {t('schema.title')} - {algorithm.name}
                </h3>
                <p className="text-[11px] text-[var(--text-muted)]">{t('schema.subtitle')}</p>
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

          {/* 模式切换 */}
          <div className="mt-4 flex items-center justify-between">
            <div className="flex rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-0.5 text-xs">
              <button
                type="button"
                onClick={() => setShowRaw(false)}
                className={`rounded-lg px-3 py-1 font-medium transition-all ${
                  !showRaw
                    ? 'bg-[var(--accent)] text-white shadow-xs'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                {t('schema.tableView')}
              </button>
              <button
                type="button"
                onClick={() => setShowRaw(true)}
                className={`flex items-center gap-1.5 rounded-lg px-3 py-1 font-medium transition-all ${
                  showRaw
                    ? 'bg-[var(--accent)] text-white shadow-xs'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                <FileJson className="h-3.5 w-3.5" />
                <span>JSON Schema</span>
              </button>
            </div>

            {showRaw && (
              <button
                type="button"
                onClick={handleCopy}
                className="flex items-center gap-1 text-xs font-medium text-[var(--accent)] hover:underline"
              >
                <Copy className="h-3.5 w-3.5" />
                <span>{copied ? t('schema.copied') : t('schema.copyJson')}</span>
              </button>
            )}
          </div>

          {/* 内容区 */}
          <div className="mt-4 max-h-96 overflow-y-auto">
            {showRaw ? (
              <pre className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 font-mono text-xs text-[var(--text-primary)]">
                {rawJsonString}
              </pre>
            ) : properties.length === 0 ? (
              <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-8 text-center text-xs text-[var(--text-muted)]">
                {t('schema.noProperties')}
              </div>
            ) : (
              <div className="overflow-hidden rounded-2xl border border-[var(--border)]">
                <table className="w-full text-left text-xs">
                  <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)]">
                    <tr>
                      <th className="px-4 py-2.5 font-semibold">{t('schema.propertyName')}</th>
                      <th className="px-4 py-2.5 font-semibold">{t('schema.type')}</th>
                      <th className="px-4 py-2.5 font-semibold">{t('schema.default')}</th>
                      <th className="px-4 py-2.5 font-semibold">{t('schema.description')}</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-[var(--border)]">
                    {properties.map((p) => (
                      <tr key={p.key} className="hover:bg-[var(--bg-secondary)]/50">
                        <td className="px-4 py-2.5 font-mono font-bold text-[var(--accent)]">
                          {p.key}
                        </td>
                        <td className="px-4 py-2.5 font-mono text-[var(--text-secondary)]">
                          {p.type}
                        </td>
                        <td className="px-4 py-2.5 font-mono text-[var(--text-muted)]">
                          {p.defaultVal}
                        </td>
                        <td className="px-4 py-2.5 text-[var(--text-secondary)]">
                          {p.description}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>

          {/* 底部按钮栏 */}
          <div className="mt-6 flex justify-end border-t border-[var(--border)] pt-4">
            <button
              type="button"
              onClick={onClose}
              className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
            >
              {t('actions.close')}
            </button>
          </div>
        </motion.div>
      </div>
    </AnimatePresence>
  )
}
