import React, { useCallback, useEffect, useState } from 'react'
import { Check, Radio, RefreshCw, Video, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { gb28181Api } from '@/lib/api'
import type { DiscoveredDevice } from '@/types'

export interface LanDiscoveryModalProps {
  isOpen: boolean
  onClose: () => void
  onSelectDevice: (device: DiscoveredDevice) => void
}

export function LanDiscoveryModal({
  isOpen,
  onClose,
  onSelectDevice,
}: LanDiscoveryModalProps): React.ReactElement | null {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  const [devices, setDevices] = useState<DiscoveredDevice[]>([])
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const handleScan = useCallback(async () => {
    setScanning(true)
    setError(null)
    try {
      const res = await gb28181Api.scanDiscovery()
      setDevices(res)
    } catch (err) {
      setError(
        err instanceof Error
          ? err.message
          : t('discovery.scanFailed', { defaultValue: '嗅探扫描失败' }),
      )
    } finally {
      setScanning(false)
    }
  }, [t])

  useEffect(() => {
    if (isOpen) {
      handleScan()
    }
  }, [isOpen, handleScan])

  useDismissStack(isOpen, onClose, { disabled: scanning })

  return (
    <AnimatePresence>
      {isOpen && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !scanning) onClose()
          }}
          className="fixed inset-0 z-[75] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="relative flex max-h-[82vh] w-full max-w-xl flex-col overflow-hidden rounded-[26px] border border-[var(--border)] bg-white shadow-[0_24px_50px_-12px_rgba(0,0,0,0.28)] dark:bg-[var(--bg-surface-solid)]"
          >
            {/* 头部 */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-6 py-4.5">
              <div className="flex items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-emerald-500/20 bg-emerald-500/10 text-emerald-600 shadow-xs dark:text-emerald-400">
                  <Radio className="h-5 w-5" />
                </div>
                <div>
                  <h3 className="text-base font-bold tracking-tight text-[var(--text-primary)] sm:text-lg">
                    {t('discovery.scanLanTitle', {
                      defaultValue: '局域网在线摄像机嗅探 (ONVIF WS-Discovery)',
                    })}
                  </h3>
                  <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                    {t('discovery.scanLanDesc', {
                      defaultValue: '通过组播协议探测局域网内的网络摄像机与 NVR 接入点',
                    })}
                  </p>
                </div>
              </div>
              <button
                onClick={onClose}
                disabled={scanning}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* 嗅探状态条 */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 px-6 py-3">
              <span className="font-mono text-xs text-[var(--text-muted)]">
                {scanning
                  ? t('discovery.scanning', { defaultValue: '正在组播嗅探中...' })
                  : t('discovery.scanSuccess', {
                      count: devices.length,
                      defaultValue: `发现 ${devices.length} 台在线摄像头`,
                    })}
              </span>
              <button
                type="button"
                onClick={handleScan}
                disabled={scanning}
                className="flex items-center gap-1.5 rounded-xl border border-[var(--border)]/80 bg-white px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] disabled:opacity-50 dark:bg-[var(--bg-surface-solid)]"
              >
                <RefreshCw className={`h-3.5 w-3.5 ${scanning ? 'animate-spin' : ''}`} />
                <span>{tc('actions.refresh')}</span>
              </button>
            </div>

            {/* 列表内容 */}
            <div className="flex-1 space-y-2.5 overflow-y-auto p-6">
              {devices.length === 0 && !scanning ? (
                <div className="flex h-44 flex-col items-center justify-center rounded-2xl border border-dashed border-[var(--border)] text-xs text-[var(--text-muted)]">
                  <Video className="mb-2 h-8 w-8 stroke-[1.5] text-[var(--text-muted)]" />
                  <span>
                    {t('discovery.noDiscoveredDevices', {
                      defaultValue: '未探测到局域网 ONVIF 摄像机设备',
                    })}
                  </span>
                </div>
              ) : (
                <div className="space-y-2.5">
                  {devices.map((dev, idx) => (
                    <div
                      key={`${dev.ip}-${dev.port}-${idx}`}
                      className="flex items-center justify-between rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 p-4 transition-all hover:border-emerald-500/40 hover:bg-white dark:hover:bg-[var(--bg-surface-solid)]"
                    >
                      <div>
                        <div className="flex items-center gap-2">
                          <span className="text-xs font-bold text-[var(--text-primary)]">
                            {dev.name ||
                              `${dev.manufacturer} ${dev.model}`.trim() ||
                              t('discovery.defaultCameraName', { defaultValue: '网络摄像头' })}
                          </span>
                          <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 font-mono text-[10px] font-semibold text-emerald-600 dark:text-emerald-400">
                            {dev.protocol.toUpperCase()}
                          </span>
                        </div>
                        <div className="font-data mt-1.5 flex items-center gap-2 text-[11px] text-[var(--text-muted)]">
                          <span>
                            IP: {dev.ip}:{dev.port}
                          </span>
                          {dev.manufacturer && (
                            <span>
                              ·{' '}
                              {t('discovery.manufacturerLabel', {
                                name: dev.manufacturer,
                                defaultValue: `厂商: ${dev.manufacturer}`,
                              })}
                            </span>
                          )}
                          {dev.model && (
                            <span>
                              ·{' '}
                              {t('discovery.modelLabel', {
                                name: dev.model,
                                defaultValue: `型号: ${dev.model}`,
                              })}
                            </span>
                          )}
                        </div>
                      </div>

                      <button
                        type="button"
                        onClick={() => {
                          onSelectDevice(dev)
                          onClose()
                        }}
                        className="flex items-center gap-1.5 rounded-xl bg-slate-900 px-3.5 py-1.5 text-xs font-semibold text-white shadow-xs transition-all hover:bg-slate-800 active:scale-95 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
                      >
                        <Check className="h-3.5 w-3.5" />
                        <span>{t('discovery.applyDevice', { defaultValue: '填入' })}</span>
                      </button>
                    </div>
                  ))}
                </div>
              )}

              {error && (
                <div className="mt-3 rounded-2xl border border-rose-500/20 bg-rose-500/10 p-3 text-xs text-rose-500">
                  {error}
                </div>
              )}
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
