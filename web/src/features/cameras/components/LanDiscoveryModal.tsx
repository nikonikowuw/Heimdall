import React, { useCallback, useEffect, useState } from 'react'
import { Check, Radio, RefreshCw, Video } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { FormErrorAlert } from '@/components/ui/FormErrorAlert'
import { ModalOverlay } from '@/components/ui/ModalOverlay'
import { CloseIconButton } from '@/components/ui/CloseIconButton'
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

  // `.then/.catch/.finally` 链：async + try/finally 内的 setState 会被
  // react-hooks/set-state-in-effect 保守判为可能同步执行（数组字面量求值
  // 阶段抛错时确实会同步进入 catch），而 `.then` 链的 setState 明确位于微任务内。
  // loading 的置位与清除都放在链内，effect 不直接 setState。
  const startScan = useCallback((): Promise<void> => {
    setScanning(true)
    return Promise.resolve()
      .then(() => gb28181Api.scanDiscovery())
      .then((res) => {
        setDevices(res)
        setError(null)
      })
      .catch((err: unknown) => {
        setError(
          err instanceof Error
            ? err.message
            : t('discovery.scanFailed', { defaultValue: '嗅探扫描失败' }),
        )
      })
      .finally(() => {
        setScanning(false)
      })
  }, [t])

  // 打开时自动扫描一次。发起动作包在微任务里，使 effect 内不出现同步 setState。
  useEffect(() => {
    if (!isOpen) return
    void Promise.resolve().then(() => startScan())
  }, [isOpen, startScan])

  return (
    <ModalOverlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabel={t('discovery.scanLanTitle', {
        defaultValue: '局域网在线摄像机嗅探 (ONVIF WS-Discovery)',
      })}
      layer="highest"
      closeDisabled={scanning}
      panelClassName="max-h-[82vh]"
    >
      {/* 头部 */}
      <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-6 py-4.5">
        <div className="flex items-center gap-3.5">
          <div className="border-status-success/20 bg-status-success/10 text-status-success flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border shadow-xs">
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
        <CloseIconButton onClick={onClose} label={t('common:close')} disabled={scanning} />
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
          onClick={() => void startScan()}
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
                className="hover:border-status-success/40 flex items-center justify-between rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 p-4 transition-all hover:bg-white dark:hover:bg-[var(--bg-surface-solid)]"
              >
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-xs font-bold text-[var(--text-primary)]">
                      {dev.name ||
                        `${dev.manufacturer} ${dev.model}`.trim() ||
                        t('discovery.defaultCameraName', { defaultValue: '网络摄像头' })}
                    </span>
                    <span className="bg-status-success/10 text-status-success rounded-full px-2 py-0.5 font-mono text-[10px] font-semibold">
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
                  className="flex items-center gap-1.5 rounded-xl bg-[var(--surface-inverse)] px-3.5 py-1.5 text-xs font-semibold text-[var(--on-inverse)] shadow-xs transition-all hover:bg-[var(--surface-inverse-hover)] active:scale-95 dark:hover:bg-white"
                >
                  <Check className="h-3.5 w-3.5" />
                  <span>{t('discovery.applyDevice', { defaultValue: '填入' })}</span>
                </button>
              </div>
            ))}
          </div>
        )}

        {error && <FormErrorAlert message={error} className="mt-3" />}
      </div>
    </ModalOverlay>
  )
}
