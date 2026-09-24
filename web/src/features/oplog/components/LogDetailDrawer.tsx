import { useEffect, useRef, useState, type ReactElement } from 'react'
import { Camera, Check, Clock, Copy, Globe, Layers, Terminal, User, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { formatTimestamp } from '@/lib/time'
import { copyToClipboard } from '@/lib/utils'
import type { OperationLog, OperationalLog } from '@/types'
import { classifyHttpStatus, getToneClasses, LEVEL_ICONS, levelTone, statusTone } from '../logTone'

export type InspectableLog =
  { kind: 'operation'; data: OperationLog } | { kind: 'operational'; data: OperationalLog }

interface LogDetailDrawerProps {
  log: InspectableLog | null
  onClose: () => void
}

function safeFormatJson(raw?: string | null): string {
  if (!raw) return ''
  try {
    const parsed = JSON.parse(raw) as unknown
    return JSON.stringify(parsed, null, 2)
  } catch {
    return raw
  }
}

function parseQueryParams(queryStr?: string | null): Array<{ key: string; value: string }> {
  if (!queryStr) return []
  const clean = queryStr.startsWith('?') ? queryStr.slice(1) : queryStr
  const params: Array<{ key: string; value: string }> = []
  for (const part of clean.split('&')) {
    if (!part) continue
    const [k, v] = part.split('=')
    params.push({
      key: decodeURIComponent(k || ''),
      value: decodeURIComponent(v || ''),
    })
  }
  return params
}

export function LogDetailDrawer({ log, onClose }: LogDetailDrawerProps): ReactElement {
  const { t, i18n } = useTranslation('oplog')
  const reducedMotion = useReducedMotion()
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const isOpen = Boolean(log)

  // 浮层关闭统一走 dismiss 栈，避免裸监听穿透关闭外层弹层
  useDismissStack(isOpen, onClose)

  // 复制反馈定时器只负责自身清理，与抽屉开关解耦
  useEffect(() => {
    return () => {
      if (copyTimeoutRef.current) {
        clearTimeout(copyTimeoutRef.current)
      }
    }
  }, [])

  async function handleCopy(key: string, text: string): Promise<void> {
    const success = await copyToClipboard(text)
    if (success) {
      if (copyTimeoutRef.current) {
        clearTimeout(copyTimeoutRef.current)
      }
      setCopiedKey(key)
      copyTimeoutRef.current = setTimeout(() => {
        setCopiedKey(null)
        copyTimeoutRef.current = null
      }, 2000)
    }
  }

  return (
    <AnimatePresence>
      {isOpen && log && (
        <div className="modal-layer modal-layer--drawer">
          {/* 背景遮罩 */}
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
            onClick={onClose}
            className="modal-scrim"
            aria-hidden="true"
          />

          {/* 侧滑抽屉主体 */}
          <motion.div
            role="dialog"
            aria-modal="true"
            aria-label={t('inspector.title')}
            initial={reducedMotion ? false : { x: '100%' }}
            animate={{ x: 0 }}
            exit={reducedMotion ? undefined : { x: '100%' }}
            transition={{ type: 'spring', damping: 30, stiffness: 350 }}
            className="modal-surface modal-surface--drawer modal-surface--drawer-wide"
          >
            {/* 抽屉顶部 Header */}
            <header className="flex shrink-0 items-center justify-between border-b border-[var(--border)] px-5 py-4">
              <div className="flex min-w-0 items-center gap-3">
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
                  <Terminal className="h-4 w-4" />
                </div>
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <h2 className="truncate text-sm font-semibold text-[var(--text-primary)]">
                      {log.kind === 'operation'
                        ? t('inspector.operationAudit')
                        : t('inspector.operationalEvent')}
                    </h2>
                    <span className="font-data rounded bg-[var(--bg-secondary)] px-1.5 py-0.5 text-[10px] text-[var(--text-muted)]">
                      #{log.data.id}
                    </span>
                  </div>
                  <p className="mt-0.5 text-[11px] text-[var(--text-muted)]">
                    {t('inspector.subtitle')}
                  </p>
                </div>
              </div>

              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => void handleCopy('all', JSON.stringify(log.data, null, 2))}
                  aria-label={t('inspector.copyAll')}
                  className="reticle-target flex h-8 items-center gap-1.5 rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] px-2.5 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:border-[var(--accent)] hover:text-[var(--accent)]"
                  title={t('inspector.copyAll')}
                >
                  {copiedKey === 'all' ? (
                    <>
                      <Check className="h-3.5 w-3.5 text-[var(--accent-green)]" />
                      <span className="text-[var(--accent-green)]">{t('inspector.copiedAll')}</span>
                    </>
                  ) : (
                    <>
                      <Copy className="h-3.5 w-3.5" />
                      <span>{t('inspector.copyAll')}</span>
                    </>
                  )}
                </button>

                <button
                  type="button"
                  onClick={onClose}
                  aria-label={t('inspector.close')}
                  className="reticle-target flex h-8 w-8 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>
            </header>

            {/* 抽屉内容滚动区域 */}
            <div className="min-h-0 flex-1 overflow-y-auto p-5">
              {log.kind === 'operation' ? (
                <OperationLogInspector
                  log={log.data}
                  copiedKey={copiedKey}
                  onCopy={handleCopy}
                  lang={i18n.language}
                />
              ) : (
                <OperationalLogInspector
                  log={log.data}
                  copiedKey={copiedKey}
                  onCopy={handleCopy}
                  lang={i18n.language}
                />
              )}
            </div>

            {/* 底部 Footer */}
            <footer className="flex shrink-0 items-center justify-between border-t border-[var(--border)] bg-[var(--bg-secondary)]/50 px-5 py-3">
              <span className="text-[11px] text-[var(--text-muted)]">
                {t('inspector.generalInfo')}
              </span>
              <button
                type="button"
                onClick={onClose}
                className="reticle-target rounded-lg border border-[var(--border-strong)] bg-[var(--bg-surface-solid)] px-4 py-1.5 text-xs font-medium text-[var(--text-primary)] shadow-xs transition-colors hover:border-[var(--accent)]"
              >
                {t('inspector.close')}
              </button>
            </footer>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}

// ─────────────────────────────────────────────────────────────
// 操作审计日志检查卡片
// ─────────────────────────────────────────────────────────────
interface OperationLogInspectorProps {
  log: OperationLog
  copiedKey: string | null
  onCopy: (key: string, text: string) => Promise<void>
  lang: string
}

function OperationLogInspector({
  log,
  copiedKey,
  onCopy,
  lang,
}: OperationLogInspectorProps): ReactElement {
  const { t } = useTranslation('oplog')
  const queryParams = parseQueryParams(log.query)
  const formattedBody = safeFormatJson(log.body)

  const statusClass = classifyHttpStatus(log.statusCode)
  const statusBadgeClass = getToneClasses(statusTone(log.statusCode)).badge

  return (
    <div className="flex flex-col gap-5">
      {/* 核心请求概述横幅 */}
      <section className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/40 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <span className="font-mono text-xs font-bold text-[var(--accent)] uppercase">
              {log.method}
            </span>
            <span
              className={`rounded-md border px-2 py-0.5 font-mono text-xs font-semibold ${statusBadgeClass}`}
            >
              {t(`status.${statusClass}`, { code: log.statusCode })}
            </span>
          </div>

          <div className="flex items-center gap-1.5 text-xs text-[var(--text-muted)]">
            <Clock className="h-3.5 w-3.5" />
            <span className="font-data font-semibold text-[var(--text-primary)]">
              {t('durationValue', { value: log.durationMs })}
            </span>
          </div>
        </div>

        {/* 完整请求路径 */}
        <div className="mt-3 flex items-center justify-between gap-2 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-3 py-2">
          <span className="font-data truncate text-xs font-medium text-[var(--text-primary)]">
            {log.path}
          </span>
          <button
            type="button"
            onClick={() => void onCopy('path', log.path)}
            className="reticle-target flex shrink-0 items-center gap-1 rounded p-1 text-[var(--text-muted)] transition-colors hover:text-[var(--accent)]"
            title={t('inspector.requestEndpoint')}
            aria-label={t('inspector.copyEndpoint')}
          >
            {copiedKey === 'path' ? (
              <Check className="h-3.5 w-3.5 text-[var(--accent-green)]" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </button>
        </div>
      </section>

      {/* 关键元数据栅格 */}
      <section className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3">
          <div className="flex items-center gap-1.5 text-[11px] text-[var(--text-muted)]">
            <User className="h-3 w-3" />
            <span>{t('columns.username')}</span>
          </div>
          <p className="mt-1 truncate text-xs font-medium text-[var(--text-primary)]">
            {log.username || t('unknown')}
          </p>
        </div>

        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3">
          <div className="flex items-center gap-1.5 text-[11px] text-[var(--text-muted)]">
            <Layers className="h-3 w-3" />
            <span>{t('columns.module')}</span>
          </div>
          <p className="font-data mt-1 truncate text-xs font-medium text-[var(--text-primary)]">
            {log.module}
          </p>
        </div>

        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3">
          <div className="flex items-center gap-1.5 text-[11px] text-[var(--text-muted)]">
            <Globe className="h-3 w-3" />
            <span>{t('columns.ip')}</span>
          </div>
          <p className="font-data mt-1 truncate text-xs font-medium text-[var(--text-primary)]">
            {log.ip || t('unknown')}
          </p>
        </div>

        <div className="col-span-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3 sm:col-span-3">
          <div className="flex items-center gap-1.5 text-[11px] text-[var(--text-muted)]">
            <Clock className="h-3 w-3" />
            <span>{t('columns.time')}</span>
          </div>
          <p className="font-data mt-1 text-xs text-[var(--text-secondary)]">
            {formatTimestamp(log.createdAt, lang)}
          </p>
        </div>
      </section>

      {/* URL 查询参数 */}
      {queryParams.length > 0 && (
        <section className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <h4 className="text-xs font-semibold text-[var(--text-primary)]">
              {t('inspector.queryParams')}
            </h4>
            <span className="text-[10px] text-[var(--text-muted)]">
              {t('inspector.paramCount', { count: queryParams.length })}
            </span>
          </div>
          <div className="overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)]">
            <table className="w-full text-left text-xs">
              <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[10px] text-[var(--text-muted)] uppercase">
                <tr>
                  <th className="px-3 py-1.5 font-medium">{t('inspector.key')}</th>
                  <th className="px-3 py-1.5 font-medium">{t('inspector.value')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border)]">
                {queryParams.map((p, idx) => (
                  <tr key={idx}>
                    <td className="font-data px-3 py-1.5 font-medium text-[var(--text-primary)]">
                      {p.key}
                    </td>
                    <td className="font-data px-3 py-1.5 break-all text-[var(--text-secondary)]">
                      {p.value}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      )}

      {/* 请求载荷 (Body JSON) */}
      <section className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <h4 className="text-xs font-semibold text-[var(--text-primary)]">
            {t('inspector.requestPayload')}
          </h4>
          {formattedBody && (
            <button
              type="button"
              onClick={() => void onCopy('body', formattedBody)}
              className="reticle-target inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium text-[var(--accent)] hover:bg-[var(--accent-soft)]"
            >
              {copiedKey === 'body' ? (
                <>
                  <Check className="h-3 w-3 text-[var(--accent-green)]" />
                  <span className="text-[var(--accent-green)]">{t('operational.copied')}</span>
                </>
              ) : (
                <>
                  <Copy className="h-3 w-3" />
                  <span>{t('operational.copyJson')}</span>
                </>
              )}
            </button>
          )}
        </div>

        <div className="overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/70 p-3">
          {formattedBody ? (
            <pre className="font-data max-h-60 overflow-auto text-[11px] leading-relaxed break-all whitespace-pre-wrap text-[var(--text-primary)]">
              {formattedBody}
            </pre>
          ) : (
            <p className="text-xs text-[var(--text-muted)] italic">{t('inspector.noPayload')}</p>
          )}
        </div>
      </section>

      {/* User Agent */}
      {log.userAgent && (
        <section className="flex flex-col gap-2">
          <h4 className="text-xs font-semibold text-[var(--text-primary)]">
            {t('inspector.userAgent')}
          </h4>
          <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3">
            <p className="font-data text-[11px] leading-relaxed break-all text-[var(--text-secondary)]">
              {log.userAgent}
            </p>
          </div>
        </section>
      )}
    </div>
  )
}

// ─────────────────────────────────────────────────────────────
// 系统运维事件日志检查卡片
// ─────────────────────────────────────────────────────────────
interface OperationalLogInspectorProps {
  log: OperationalLog
  copiedKey: string | null
  onCopy: (key: string, text: string) => Promise<void>
  lang: string
}

function OperationalLogInspector({
  log,
  copiedKey,
  onCopy,
  lang,
}: OperationalLogInspectorProps): ReactElement {
  const { t } = useTranslation('oplog')
  const formattedExtra = safeFormatJson(log.extraJson)

  const LevelIcon = LEVEL_ICONS[log.level]
  const levelBadgeClass = getToneClasses(levelTone(log.level)).badge

  return (
    <div className="flex flex-col gap-5">
      {/* 顶部事件概况 */}
      <section className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/40 p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <span
              className={`inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-bold uppercase ${levelBadgeClass}`}
            >
              <LevelIcon className="h-3.5 w-3.5" />
              {t(`operational.levels.${log.level}`)}
            </span>

            <span className="font-data rounded-md border border-[var(--border)] bg-[var(--bg-surface-solid)] px-2 py-0.5 text-xs font-medium text-[var(--text-primary)]">
              {log.target}
            </span>
          </div>

          <span className="font-data text-xs text-[var(--text-muted)]">
            {formatTimestamp(log.tsMs, lang)}
          </span>
        </div>

        {/* 事件名 */}
        <div className="mt-3 rounded-lg border border-[var(--border)] bg-[var(--bg-surface-solid)] px-3 py-2">
          <div className="flex items-center justify-between gap-2">
            <span className="font-data text-xs font-semibold text-[var(--text-primary)]">
              {log.event}
            </span>
            <button
              type="button"
              onClick={() => void onCopy('event', log.event)}
              className="reticle-target flex shrink-0 items-center gap-1 rounded p-1 text-[var(--text-muted)] transition-colors hover:text-[var(--accent)]"
              title={t('inspector.copyEvent')}
              aria-label={t('inspector.copyEvent')}
            >
              {copiedKey === 'event' ? (
                <Check className="h-3.5 w-3.5 text-[var(--accent-green)]" />
              ) : (
                <Copy className="h-3.5 w-3.5" />
              )}
            </button>
          </div>
        </div>
      </section>

      {/* 详细描述 */}
      <section className="flex flex-col gap-2">
        <h4 className="text-xs font-semibold text-[var(--text-primary)]">
          {t('inspector.eventDetails')}
        </h4>
        <div className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3.5">
          <p className="text-xs leading-relaxed whitespace-pre-wrap text-[var(--text-primary)]">
            {log.message}
          </p>
        </div>
      </section>

      {/* 关联摄像头信息 (如果有) */}
      {log.cameraId && (
        <section className="rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-3.5">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Camera className="h-4 w-4 text-[var(--accent)]" />
              <span className="text-xs font-medium text-[var(--text-muted)]">
                {t('operational.columns.camera')}
              </span>
            </div>
            <span className="font-data text-xs font-semibold text-[var(--accent)]">
              {log.cameraId}
            </span>
          </div>
        </section>
      )}

      {/* 元数据 JSON 载荷 */}
      <section className="flex flex-col gap-2">
        <div className="flex items-center justify-between">
          <h4 className="text-xs font-semibold text-[var(--text-primary)]">
            {t('inspector.extraJson')}
          </h4>
          {formattedExtra && (
            <button
              type="button"
              onClick={() => void onCopy('extra', formattedExtra)}
              className="reticle-target inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] font-medium text-[var(--accent)] hover:bg-[var(--accent-soft)]"
            >
              {copiedKey === 'extra' ? (
                <>
                  <Check className="h-3 w-3 text-[var(--accent-green)]" />
                  <span className="text-[var(--accent-green)]">{t('operational.copied')}</span>
                </>
              ) : (
                <>
                  <Copy className="h-3 w-3" />
                  <span>{t('operational.copyJson')}</span>
                </>
              )}
            </button>
          )}
        </div>

        <div className="overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]/70 p-3">
          {formattedExtra ? (
            <pre className="font-data max-h-72 overflow-auto text-[11px] leading-relaxed break-all whitespace-pre-wrap text-[var(--text-primary)]">
              {formattedExtra}
            </pre>
          ) : (
            <p className="text-xs text-[var(--text-muted)] italic">{t('inspector.noExtra')}</p>
          )}
        </div>
      </section>
    </div>
  )
}
