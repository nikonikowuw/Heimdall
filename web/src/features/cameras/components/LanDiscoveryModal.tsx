import React, { useState, useEffect, useCallback } from 'react'
import { motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { Check, Radio, RefreshCw, Video, X } from 'lucide-react'
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

  if (!isOpen) return null

  return (
    <div
      onClick={(e) => {
        if (e.target === e.currentTarget && !scanning) onClose()
      }}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.95 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.95 }}
        className="frosted-glass relative flex max-h-[80vh] w-full max-w-xl flex-col rounded-2xl border border-[var(--border)] bg-[var(--surface-elevated)] p-6 shadow-2xl"
      >
        <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent)]/10 text-[var(--accent)]">
              <Radio className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-bold text-[var(--text-primary)]">
                {t('discovery.scanLanTitle', {
                  defaultValue: '局域网在线摄像机嗅探 (ONVIF WS-Discovery)',
                })}
              </h3>
              <p className="text-xs text-[var(--text-secondary)]">
                {t('discovery.scanLanDesc', {
                  defaultValue: '通过组播协议探测局域网内的网络摄像机与 NVR 接入点',
                })}
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            disabled={scanning}
            className="rounded-lg p-1.5 text-[var(--text-muted)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="flex items-center justify-between py-3">
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
            className="flex items-center gap-1.5 rounded-lg border border-[var(--border)] px-2.5 py-1 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--surface-hover)] disabled:opacity-50"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${scanning ? 'animate-spin' : ''}`} />
            <span>{tc('actions.refresh')}</span>
          </button>
        </div>

        <div className="flex-1 overflow-y-auto pr-1">
          {devices.length === 0 && !scanning ? (
            <div className="flex h-40 flex-col items-center justify-center text-xs text-[var(--text-muted)]">
              <Video className="mb-2 h-8 w-8 stroke-[1.5] text-[var(--text-muted)]" />
              <span>
                {t('discovery.noDiscoveredDevices', {
                  defaultValue: '未探测到局域网 ONVIF 摄像机设备',
                })}
              </span>
            </div>
          ) : (
            <div className="space-y-2">
              {devices.map((dev, idx) => (
                <div
                  key={`${dev.ip}-${dev.port}-${idx}`}
                  className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--surface)] p-3 transition-colors hover:border-[var(--accent)]/50"
                >
                  <div>
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-semibold text-[var(--text-primary)]">
                        {dev.name ||
                          `${dev.manufacturer} ${dev.model}`.trim() ||
                          t('discovery.defaultCameraName', { defaultValue: '网络摄像头' })}
                      </span>
                      <span className="py-0.2 rounded bg-cyan-500/10 px-1.5 font-mono text-[10px] text-cyan-400">
                        {dev.protocol.toUpperCase()}
                      </span>
                    </div>
                    <div className="mt-1 flex items-center gap-2 font-mono text-[11px] text-[var(--text-muted)]">
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
                    className="flex items-center gap-1 rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white hover:opacity-90"
                  >
                    <Check className="h-3.5 w-3.5" />
                    <span>{t('discovery.applyDevice', { defaultValue: '填入' })}</span>
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {error && (
          <div className="mt-3 rounded-xl border border-rose-500/20 bg-rose-500/10 p-2.5 text-xs text-rose-400">
            {error}
          </div>
        )}
      </motion.div>
    </div>
  )
}
