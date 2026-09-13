import React, { useEffect, useState } from 'react'
import { AlertCircle, Check, Loader2, Radio, Sparkles, Video, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { StreamModeSelector } from '@/components/StreamModeSelector'
import { cameraApi, gb28181Api } from '@/lib/api'
import type {
  Camera,
  SubStreamCandidate,
  StreamMode,
  Gb28181Device,
  DiscoveredDevice,
} from '@/types'
import { LanDiscoveryModal } from './LanDiscoveryModal'

export interface CameraModalProps {
  isOpen: boolean
  camera?: Camera | null
  onClose: () => void
  onSuccess: (camera: Camera) => void
}

export function CameraModal({
  isOpen,
  camera = null,
  onClose,
  onSuccess,
}: CameraModalProps): React.ReactElement | null {
  const { t } = useTranslation('camera')
  const { t: tc } = useTranslation('common')

  const isEdit = Boolean(camera)

  const [protocol, setProtocol] = useState<'rtsp' | 'gb28181'>('rtsp')
  const [name, setName] = useState('')
  const [mainUrl, setMainUrl] = useState('')
  const [subUrl, setSubUrl] = useState('')
  const [streamMode, setStreamMode] = useState<StreamMode>('auto')
  const [remark, setRemark] = useState('')
  const [subCandidates, setSubCandidates] = useState<SubStreamCandidate[]>([])
  const [isDeducing, setIsDeducing] = useState(false)
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  // GB28181 状态
  const [gbDevices, setGbDevices] = useState<Gb28181Device[]>([])
  const [selectedGbDevice, setSelectedGbDevice] = useState<string>('')
  const [selectedGbChannel, setSelectedGbChannel] = useState<string>('')
  const [isLanScanOpen, setIsLanScanOpen] = useState(false)

  // 当弹窗打开或切换目标 camera 时，重置/初始化表单数据
  useEffect(() => {
    if (isOpen) {
      if (camera) {
        setProtocol(camera.protocol === 'gb28181' ? 'gb28181' : 'rtsp')
        setName(camera.name || '')
        setMainUrl(camera.rtspUrl || '')
        setSubUrl(camera.subRtspUrl || '')
        setStreamMode(camera.streamMode || 'auto')
        setRemark(camera.remark || '')
        setSelectedGbDevice(camera.gb28181DeviceId || '')
        setSelectedGbChannel(camera.gb28181ChannelId || '')
      } else {
        setProtocol('rtsp')
        setName('')
        setMainUrl('')
        setSubUrl('')
        setStreamMode('auto')
        setRemark('')
        setSelectedGbDevice('')
        setSelectedGbChannel('')
      }
      setSubCandidates([])
      setErrorMsg(null)

      // 拉取 GB28181 设备树
      gb28181Api
        .listDevices()
        .then((devs) => {
          setGbDevices(devs)
          if (!camera && devs.length > 0) {
            setSelectedGbDevice(devs[0].deviceId)
            if (devs[0].channels && devs[0].channels.length > 0) {
              setSelectedGbChannel(devs[0].channels[0].channelId)
            }
          }
        })
        .catch(() => {})
    }
  }, [isOpen, camera])

  // 当 GB28181 设备或通道选择变化时自动构造 URL
  const handleGbDeviceChange = (devId: string) => {
    setSelectedGbDevice(devId)
    const dev = gbDevices.find((d) => d.deviceId === devId)
    if (dev && dev.channels && dev.channels.length > 0) {
      const ch = dev.channels[0]
      setSelectedGbChannel(ch.channelId)
      setMainUrl(`gb28181://${devId}/${ch.channelId}`)
      if (!name) {
        setName(ch.name || `${dev.name || devId}-CH1`)
      }
    } else {
      setSelectedGbChannel('')
      setMainUrl(`gb28181://${devId}/`)
    }
  }

  const handleGbChannelChange = (chId: string) => {
    setSelectedGbChannel(chId)
    setMainUrl(`gb28181://${selectedGbDevice}/${chId}`)
    const dev = gbDevices.find((d) => d.deviceId === selectedGbDevice)
    const ch = dev?.channels?.find((c) => c.channelId === chId)
    if (ch && (!name || name.startsWith(dev?.name || ''))) {
      const devPrefix = dev?.name || selectedGbDevice || 'Device'
      setName(ch.name || `${devPrefix}-${chId.slice(-4)}`)
    }
  }

  // ESC 快捷键关闭
  useEffect(() => {
    if (!isOpen) return
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !isSubmitting) {
        onClose()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [isOpen, isSubmitting, onClose])

  if (!isOpen) return null

  const handleDeduce = async (url: string) => {
    const trimmed = url.trim()
    if (!trimmed || protocol === 'gb28181') return
    setIsDeducing(true)
    try {
      const candidates = await cameraApi.deduceSubStream(trimmed)
      setSubCandidates(candidates)
      if (candidates.length > 0 && !subUrl) {
        setSubUrl(candidates[0].subUrl)
      }
    } catch {
      // 容错处理：推导失败不阻断用户手动输入
    } finally {
      setIsDeducing(false)
    }
  }

  const handleSelectDiscovered = (dev: DiscoveredDevice) => {
    if (dev.rtspUrl) {
      setMainUrl(dev.rtspUrl)
    } else {
      setMainUrl(`rtsp://admin:admin@${dev.ip}:${dev.port}/h264`)
    }
    if (!name) {
      setName(
        dev.name ||
          `${dev.manufacturer} ${dev.model}`.trim() ||
          t('discovery.defaultCameraName', { defaultValue: '网络摄像头' }),
      )
    }
  }

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setErrorMsg(null)

    const trimmedName = name.trim()
    const trimmedMainUrl = mainUrl.trim()
    const trimmedSubUrl = subUrl.trim()
    const trimmedRemark = remark.trim()

    if (!trimmedName) {
      setErrorMsg(t('manage.nameRequired', { defaultValue: '设备名称不能为空' }))
      return
    }
    if (!trimmedMainUrl) {
      setErrorMsg(t('manage.mainRtspRequired', { defaultValue: '主码流地址不能为空' }))
      return
    }

    setIsSubmitting(true)
    try {
      const isGb = protocol === 'gb28181'
      const gb28181DeviceId = isGb ? selectedGbDevice || undefined : undefined
      const gb28181ChannelId = isGb ? selectedGbChannel || undefined : undefined

      if (isEdit && camera) {
        const updated = await cameraApi.update(camera.cameraId, {
          name: trimmedName,
          rtspUrl: trimmedMainUrl,
          subRtspUrl: trimmedSubUrl,
          streamMode,
          remark: trimmedRemark || undefined,
          gb28181DeviceId,
          gb28181ChannelId,
        })
        onSuccess(updated)
        onClose()
      } else {
        let subRtspUrl: string | undefined
        if (trimmedSubUrl !== '') {
          subRtspUrl = trimmedSubUrl
        } else if (streamMode === 'main') {
          subRtspUrl = ''
        }
        const created = await cameraApi.create({
          name: trimmedName,
          protocol,
          rtspUrl: trimmedMainUrl,
          subRtspUrl,
          streamMode,
          remark: trimmedRemark || undefined,
          gb28181DeviceId,
          gb28181ChannelId,
        })
        onSuccess(created)
        onClose()
      }
    } catch (err) {
      const msg =
        err instanceof Error
          ? err.message
          : t('manage.saveFailed', { defaultValue: '保存失败，请检查流地址格式或网络连接' })
      setErrorMsg(msg)
    } finally {
      setIsSubmitting(false)
    }
  }

  const currentGbDev = gbDevices.find((d) => d.deviceId === selectedGbDevice)

  let submitButtonText = t('manage.saveAndProbe')
  if (isSubmitting) {
    submitButtonText = t('manage.saving')
  } else if (isEdit) {
    submitButtonText = tc('actions.save')
  }

  return (
    <>
      <div
        onClick={(e) => {
          if (e.target === e.currentTarget && !isSubmitting) {
            onClose()
          }
        }}
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs"
      >
        <div className="frosted-glass relative w-full max-w-lg rounded-2xl border border-[var(--border)] p-6 shadow-2xl transition-all">
          {/* 关闭按钮 */}
          <button
            type="button"
            onClick={onClose}
            disabled={isSubmitting}
            className="absolute top-5 right-5 rounded-lg p-1 text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
          >
            <X className="h-4 w-4" />
          </button>

          {/* 头部标题与描述 */}
          <div className="flex items-start gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
              <Video className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-semibold text-[var(--text-primary)]">
                {isEdit ? t('manage.editCameraTitle') : t('manage.addCameraTitle')}
              </h3>
              <p className="mt-1 text-xs text-[var(--text-muted)]">
                {isEdit ? t('manage.editCameraDesc') : t('manage.addCameraDesc')}
              </p>
            </div>
          </div>

          {/* 协议切换 Pills */}
          {!isEdit && (
            <div className="mt-4 flex items-center gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] p-1">
              <button
                type="button"
                onClick={() => {
                  setProtocol('rtsp')
                  setMainUrl('')
                }}
                className={`flex-1 rounded-lg py-1.5 text-xs font-semibold transition-all ${
                  protocol === 'rtsp'
                    ? 'bg-[var(--accent)] text-white shadow-xs'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                {t('protocol.rtspAccess', { defaultValue: 'RTSP 协议接入' })}
              </button>
              <button
                type="button"
                onClick={() => {
                  setProtocol('gb28181')
                  if (selectedGbDevice && selectedGbChannel) {
                    setMainUrl(`gb28181://${selectedGbDevice}/${selectedGbChannel}`)
                  }
                }}
                className={`flex-1 rounded-lg py-1.5 text-xs font-semibold transition-all ${
                  protocol === 'gb28181'
                    ? 'bg-cyan-600 text-white shadow-xs'
                    : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                }`}
              >
                {t('protocol.gb28181Access', { defaultValue: '国标 GB28181 接入' })}
              </button>
            </div>
          )}

          {/* 错误提示条 */}
          {errorMsg && (
            <div className="mt-4 flex items-center gap-2 rounded-xl border border-rose-500/30 bg-rose-500/10 p-3 text-xs text-rose-400">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{errorMsg}</span>
            </div>
          )}

          <form onSubmit={handleSubmit} className="mt-4 flex flex-col gap-4">
            {/* GB28181 设备树选择联动 */}
            {protocol === 'gb28181' ? (
              <div className="space-y-3 rounded-xl border border-cyan-500/30 bg-cyan-500/5 p-3.5">
                <div>
                  <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                    {t('protocol.selectGbDevice', { defaultValue: '选择注册的国标设备' })}
                  </label>
                  {gbDevices.length === 0 ? (
                    <p className="text-xs text-[var(--text-muted)]">
                      {t('protocol.noGbDevices', {
                        defaultValue:
                          '当前暂无注册上线的国标设备。请先让 IPC / NVR 对接到本机 SIP 服务器。',
                      })}
                    </p>
                  ) : (
                    <select
                      value={selectedGbDevice}
                      onChange={(e) => handleGbDeviceChange(e.target.value)}
                      className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none"
                    >
                      {gbDevices.map((d) => (
                        <option key={d.deviceId} value={d.deviceId}>
                          {d.name || d.deviceId} ({d.ipAddr})
                        </option>
                      ))}
                    </select>
                  )}
                </div>

                {currentGbDev && currentGbDev.channels && (
                  <div>
                    <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
                      {t('protocol.selectGbChannel', { defaultValue: '选择视频通道' })}
                    </label>
                    <select
                      value={selectedGbChannel}
                      onChange={(e) => handleGbChannelChange(e.target.value)}
                      className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3 py-2 text-xs text-[var(--text-primary)] outline-none"
                    >
                      {currentGbDev.channels.map((ch) => (
                        <option key={ch.channelId} value={ch.channelId}>
                          {t('protocol.channelFormat', {
                            name: ch.name || ch.channelId,
                            status: ch.status,
                            defaultValue: `${ch.name || ch.channelId} [${ch.status}]`,
                          })}
                        </option>
                      ))}
                    </select>
                  </div>
                )}
              </div>
            ) : null}

            {/* 设备名称 */}
            <div>
              <label className="text-xs font-medium text-[var(--text-secondary)]">
                {t('manage.name')} <span className="text-rose-500">*</span>
              </label>
              <input
                type="text"
                required
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t('manage.namePlaceholder')}
                className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
              />
            </div>

            {/* 主码流地址 */}
            <div>
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {protocol === 'gb28181'
                    ? t('protocol.gbUri', { defaultValue: '国标 URI 地址' })
                    : t('manage.mainRtspUrl')}{' '}
                  <span className="text-rose-500">*</span>
                </label>
                {protocol === 'rtsp' && (
                  <div className="flex items-center gap-2">
                    <button
                      type="button"
                      onClick={() => setIsLanScanOpen(true)}
                      className="flex items-center gap-1 text-[11px] text-[var(--accent)] hover:underline"
                    >
                      <Radio className="h-3 w-3" />
                      <span>{t('discovery.scanLan', { defaultValue: '局域网嗅探' })}</span>
                    </button>
                    {isDeducing && (
                      <span className="flex items-center gap-1 text-[10px] text-cyan-400">
                        <Loader2 className="h-3 w-3 animate-spin" />
                        <span>{t('discovery.deducing', { defaultValue: '推导中...' })}</span>
                      </span>
                    )}
                  </div>
                )}
              </div>
              <input
                type="text"
                required
                value={mainUrl}
                onChange={(e) => setMainUrl(e.target.value)}
                onBlur={() => handleDeduce(mainUrl)}
                readOnly={protocol === 'gb28181'}
                placeholder={
                  protocol === 'gb28181'
                    ? 'gb28181://{deviceId}/{channelId}'
                    : t('manage.mainRtspPlaceholder')
                }
                className={`mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 font-mono text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden ${
                  protocol === 'gb28181' ? 'opacity-80' : ''
                }`}
              />
            </div>

            {/* 子码流 RTSP 地址 (仅 RTSP 模式支持) */}
            {protocol === 'rtsp' && (
              <div>
                <div className="flex items-center justify-between">
                  <label className="text-xs font-medium text-[var(--text-secondary)]">
                    {t('manage.subRtspUrl')}
                  </label>
                  {subCandidates.length > 0 && (
                    <span className="flex items-center gap-1 text-[10px] font-medium text-cyan-400">
                      <Sparkles className="h-3 w-3" />
                      <span>{t('manage.subCandidatesHint')}</span>
                    </span>
                  )}
                </div>
                <input
                  type="text"
                  value={subUrl}
                  onChange={(e) => setSubUrl(e.target.value)}
                  placeholder={t('manage.subRtspPlaceholder')}
                  className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 font-mono text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
                />
                <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                  {t('manage.subRtspHint')}
                </p>

                {/* 子码流候选芯片预设 */}
                {subCandidates.length > 0 && (
                  <div className="mt-2 flex flex-wrap gap-1.5">
                    {subCandidates.map((c, i) => {
                      const isSelected = subUrl === c.subUrl
                      return (
                        <button
                          type="button"
                          key={i}
                          onClick={() => setSubUrl(c.subUrl)}
                          className={`flex items-center gap-1 rounded-lg px-2.5 py-1 font-mono text-[10px] transition-all ${
                            isSelected
                              ? 'border border-cyan-500/40 bg-cyan-500/20 font-semibold text-cyan-400'
                              : 'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--text-primary)]'
                          }`}
                          title={c.description}
                        >
                          {isSelected && <Check className="h-3 w-3 text-cyan-400" />}
                          <span>
                            {c.brand}: {c.description}
                          </span>
                        </button>
                      )
                    })}
                  </div>
                )}
              </div>
            )}

            {/* AI 分析码流选择 */}
            <div>
              <label className="text-xs font-medium text-[var(--text-secondary)]">
                {t('manage.streamMode')}
              </label>
              <div className="mt-1">
                <StreamModeSelector value={streamMode} onChange={setStreamMode} />
              </div>
            </div>

            {/* 备注 */}
            <div>
              <label className="text-xs font-medium text-[var(--text-secondary)]">
                {t('manage.remark')}
              </label>
              <input
                type="text"
                value={remark}
                onChange={(e) => setRemark(e.target.value)}
                placeholder={t('manage.remarkPlaceholder')}
                className="mt-1 w-full rounded-xl border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-2 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-colors focus:border-[var(--accent)] focus:outline-hidden"
              />
            </div>

            {/* 底部操作按钮 */}
            <div className="mt-2 flex items-center justify-end gap-3 pt-2">
              <button
                type="button"
                onClick={onClose}
                disabled={isSubmitting}
                className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] disabled:opacity-50"
              >
                {t('manage.cancel')}
              </button>
              <button
                type="submit"
                disabled={isSubmitting}
                className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-5 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 active:scale-95 disabled:opacity-50"
              >
                {isSubmitting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                <span>{submitButtonText}</span>
              </button>
            </div>
          </form>
        </div>
      </div>

      <LanDiscoveryModal
        isOpen={isLanScanOpen}
        onClose={() => setIsLanScanOpen(false)}
        onSelectDevice={handleSelectDiscovered}
      />
    </>
  )
}
