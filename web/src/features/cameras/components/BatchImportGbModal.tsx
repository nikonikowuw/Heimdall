import React, { useMemo, useState } from 'react'
import { Check, CheckCircle2, Loader2, Radio, Server, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { gb28181Api } from '@/lib/api'
import type { Gb28181Channel, Gb28181Device, ImportGbChannelItem } from '@/types'

export interface BatchImportGbModalProps {
  isOpen: boolean
  onClose: () => void
  onSuccess: () => void
  devices: Gb28181Device[]
}

function getChannelKey(deviceId: string, channelId: string): string {
  return `${deviceId}/${channelId}`
}

export function BatchImportGbModal({
  isOpen,
  onClose,
  onSuccess,
  devices,
}: BatchImportGbModalProps): React.ReactElement | null {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  // 提取所有未纳管通道
  const unmanagedList = useMemo(() => {
    const list: { device: Gb28181Device; channel: Gb28181Channel }[] = []
    for (const dev of devices) {
      for (const ch of dev.channels || []) {
        if (!ch.isImported) {
          list.push({ device: dev, channel: ch })
        }
      }
    }
    return list
  }, [devices])

  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(() => {
    return new Set(
      unmanagedList.map((item) => getChannelKey(item.device.deviceId, item.channel.channelId)),
    )
  })
  const [streamModes, setStreamModes] = useState<Record<string, 'auto' | 'main' | 'sub'>>({})
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  useDismissStack(isOpen, onClose, { disabled: isSubmitting })

  React.useEffect(() => {
    setSelectedKeys(
      new Set(
        unmanagedList.map((item) => getChannelKey(item.device.deviceId, item.channel.channelId)),
      ),
    )
  }, [unmanagedList])

  const toggleSelect = (key: string) => {
    setSelectedKeys((prev) => {
      const next = new Set(prev)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }

  const toggleSelectAll = () => {
    if (selectedKeys.size === unmanagedList.length) {
      setSelectedKeys(new Set())
    } else {
      setSelectedKeys(
        new Set(
          unmanagedList.map((item) => getChannelKey(item.device.deviceId, item.channel.channelId)),
        ),
      )
    }
  }

  const handleImport = async () => {
    if (selectedKeys.size === 0) return
    setIsSubmitting(true)
    setErrorMsg(null)

    const items: ImportGbChannelItem[] = []
    for (const item of unmanagedList) {
      const key = getChannelKey(item.device.deviceId, item.channel.channelId)
      if (selectedKeys.has(key)) {
        items.push({
          deviceId: item.device.deviceId,
          channelId: item.channel.channelId,
          name: item.channel.name || `${item.device.name}-${item.channel.channelId.slice(-4)}`,
          streamMode: streamModes[key] || 'auto',
        })
      }
    }

    try {
      await gb28181Api.batchImportChannels({ channels: items })
      onSuccess()
      onClose()
    } catch (err) {
      setErrorMsg(
        err instanceof Error
          ? err.message
          : t('discovery.importFailed', { defaultValue: '批量导入失败' }),
      )
    } finally {
      setIsSubmitting(false)
    }
  }

  return (
    <AnimatePresence>
      {isOpen && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !isSubmitting) onClose()
          }}
          className="modal-backdrop modal-backdrop--top"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="modal-surface modal-surface--medium max-h-[85vh]"
          >
            {/* Header */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-6 py-4.5">
              <div className="flex items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-cyan-500/20 bg-cyan-500/10 text-cyan-600 shadow-xs dark:text-cyan-400">
                  <Radio className="h-5 w-5" />
                </div>
                <div>
                  <h3 className="text-base font-bold tracking-tight text-[var(--text-primary)] sm:text-lg">
                    {t('discovery.drawerTitle', { defaultValue: '批量纳管国标通道' })}
                  </h3>
                  <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                    {t('discovery.drawerDesc', {
                      count: unmanagedList.length,
                      defaultValue: `已发现 ${unmanagedList.length} 个未纳管视频通道，勾选后一键录入系统资产池`,
                    })}
                  </p>
                </div>
              </div>
              <button
                onClick={onClose}
                disabled={isSubmitting}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* Action Bar */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 px-6 py-3">
              <button
                type="button"
                onClick={toggleSelectAll}
                className="flex items-center gap-2 text-xs font-semibold text-[var(--text-secondary)] transition-colors hover:text-[var(--text-primary)]"
              >
                <div
                  className={`flex h-4 w-4 items-center justify-center rounded-md border transition-colors ${
                    selectedKeys.size === unmanagedList.length && unmanagedList.length > 0
                      ? 'border-cyan-600 bg-cyan-600 text-white dark:border-cyan-500 dark:bg-cyan-500'
                      : 'border-[var(--border)] bg-white dark:bg-[var(--bg-surface-solid)]'
                  }`}
                >
                  {selectedKeys.size === unmanagedList.length && unmanagedList.length > 0 && (
                    <Check className="h-3 w-3 stroke-[3]" />
                  )}
                </div>
                <span>
                  {selectedKeys.size === unmanagedList.length
                    ? t('discovery.unselectAll', { defaultValue: '取消全选' })
                    : t('discovery.selectAll', { defaultValue: '全选' })}
                </span>
              </button>
              <span className="font-mono text-xs text-[var(--text-muted)]">
                {t('discovery.selectedCount', {
                  selected: selectedKeys.size,
                  total: unmanagedList.length,
                  defaultValue: `已选: ${selectedKeys.size} / ${unmanagedList.length}`,
                })}
              </span>
            </div>

            {/* Channel List */}
            <div className="flex-1 space-y-2.5 overflow-y-auto p-6">
              {unmanagedList.length === 0 ? (
                <div className="flex h-44 flex-col items-center justify-center rounded-2xl border border-dashed border-[var(--border)] text-xs text-[var(--text-muted)]">
                  <Server className="mb-2 h-8 w-8 stroke-[1.5] text-[var(--text-muted)]" />
                  <span>{t('discovery.noChannels', { defaultValue: '暂无待纳管的国标通道' })}</span>
                </div>
              ) : (
                <div className="space-y-2.5">
                  {unmanagedList.map(({ device, channel }) => {
                    const key = getChannelKey(device.deviceId, channel.channelId)
                    const isChecked = selectedKeys.has(key)
                    const currentMode = streamModes[key] || 'auto'

                    return (
                      <div
                        key={key}
                        onClick={() => toggleSelect(key)}
                        className={`flex cursor-pointer items-center justify-between rounded-2xl border p-4 transition-all ${
                          isChecked
                            ? 'border-cyan-500/50 bg-cyan-500/10'
                            : 'border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 hover:border-[var(--border-strong)] hover:bg-white dark:hover:bg-[var(--bg-surface-solid)]'
                        }`}
                      >
                        <div className="flex items-center gap-3.5">
                          <div
                            className={`flex h-4 w-4 shrink-0 items-center justify-center rounded-md border transition-colors ${
                              isChecked
                                ? 'border-cyan-600 bg-cyan-600 text-white dark:border-cyan-500 dark:bg-cyan-500'
                                : 'border-[var(--border)] bg-white dark:bg-[var(--bg-surface-solid)]'
                            }`}
                          >
                            {isChecked && <Check className="h-3 w-3 stroke-[3]" />}
                          </div>
                          <div>
                            <div className="flex items-center gap-2">
                              <span className="text-xs font-bold text-[var(--text-primary)]">
                                {channel.name ||
                                  t('discovery.unnamedChannel', { defaultValue: '未命名国标通道' })}
                              </span>
                              <span
                                className={`rounded-full px-2 py-0.5 font-mono text-[10px] font-semibold ${
                                  channel.status === 'ON'
                                    ? 'bg-emerald-500/10 text-emerald-600 dark:text-emerald-400'
                                    : 'bg-slate-500/10 text-[var(--text-muted)]'
                                }`}
                              >
                                {channel.status || 'ON'}
                              </span>
                            </div>
                            <div className="font-data mt-1 flex items-center gap-2 text-[11px] text-[var(--text-muted)]">
                              <span>
                                {t('discovery.devicePrefix', {
                                  name: device.name || device.deviceId,
                                  defaultValue: `设备: ${device.name || device.deviceId}`,
                                })}
                              </span>
                              <span>·</span>
                              <span>
                                {t('discovery.channelPrefix', {
                                  id: channel.channelId,
                                  defaultValue: `通道: ${channel.channelId}`,
                                })}
                              </span>
                            </div>
                          </div>
                        </div>

                        <div
                          onClick={(e) => e.stopPropagation()}
                          className="flex items-center gap-2"
                        >
                          <select
                            value={currentMode}
                            onChange={(e) => {
                              const val = e.target.value as 'auto' | 'main' | 'sub'
                              setStreamModes((prev) => ({ ...prev, [key]: val }))
                            }}
                            className="rounded-xl border border-[var(--border)]/80 bg-white px-3 py-1.5 font-mono text-xs text-[var(--text-secondary)] shadow-xs outline-none focus:border-cyan-500 dark:bg-[var(--bg-surface-solid)]"
                          >
                            <option value="auto">
                              {t('discovery.streamModeAutoOption', {
                                defaultValue: '自动码流 (推荐)',
                              })}
                            </option>
                            <option value="main">
                              {t('discovery.streamModeMainOption', { defaultValue: '主码流分析' })}
                            </option>
                            <option value="sub">
                              {t('discovery.streamModeSubOption', { defaultValue: '子码流分析' })}
                            </option>
                          </select>
                        </div>
                      </div>
                    )
                  })}
                </div>
              )}
            </div>

            {errorMsg && (
              <div className="mx-6 mb-4 rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3 text-xs text-[var(--status-danger)]">
                {errorMsg}
              </div>
            )}

            {/* Footer actions */}
            <div className="flex shrink-0 items-center justify-end gap-2.5 border-t border-[var(--border)]/70 bg-[var(--bg-secondary)]/20 px-6 py-4">
              <button
                type="button"
                onClick={onClose}
                disabled={isSubmitting}
                className="rounded-xl border border-[var(--border)]/80 bg-white px-4 py-2 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] disabled:opacity-50 dark:bg-[var(--bg-surface-solid)]"
              >
                {tc('actions.cancel')}
              </button>
              <button
                type="button"
                onClick={handleImport}
                disabled={isSubmitting || selectedKeys.size === 0}
                className="flex min-h-9 items-center gap-2 rounded-xl bg-cyan-600 px-5 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:bg-cyan-700 active:scale-95 disabled:opacity-50"
              >
                {isSubmitting ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                ) : (
                  <CheckCircle2 className="h-3.5 w-3.5" />
                )}
                <span>
                  {t('discovery.importSelected', {
                    count: selectedKeys.size,
                    defaultValue: `导入选中的通道 (${selectedKeys.size})`,
                  })}
                </span>
              </button>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
