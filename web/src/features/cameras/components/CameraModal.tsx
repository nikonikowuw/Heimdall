import React, { useEffect, useId, useState } from 'react'
import {
  AlertCircle,
  Camera as CameraIcon,
  Check,
  ChevronDown,
  Loader2,
  Radio,
  Sparkles,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { StreamModeSelector } from '@/components/StreamModeSelector'
import { ModalFormHeader } from '@/components/ui/ModalFormHeader'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { cameraApi, gb28181Api } from '@/lib/api'
import type {
  Camera,
  DiscoveredDevice,
  Gb28181Device,
  StreamMode,
  SubStreamCandidate,
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
  const titleId = useId()
  const descriptionId = useId()
  const nameFieldId = useId()
  const remarkFieldId = useId()
  const deviceFieldId = useId()
  const channelFieldId = useId()
  const mainUrlFieldId = useId()
  const subUrlFieldId = useId()

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

  // ESC 快捷键由 dismiss stack 统一管理
  useDismissStack(isOpen, onClose, { disabled: isSubmitting })

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
      <AnimatePresence>
        {isOpen && (
          <div
            onClick={(e) => {
              if (e.target === e.currentTarget && !isSubmitting) {
                onClose()
              }
            }}
            className="modal-backdrop modal-backdrop--top"
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.96, y: 10 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.96, y: 10 }}
              transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
              role="dialog"
              aria-modal="true"
              aria-labelledby={titleId}
              aria-describedby={descriptionId}
              className="modal-surface modal-surface--form"
            >
              <ModalFormHeader
                icon={CameraIcon}
                title={isEdit ? t('manage.editCameraTitle') : t('manage.addCameraTitle')}
                titleId={titleId}
                description={isEdit ? t('manage.editCameraDesc') : t('manage.addCameraDesc')}
                descriptionId={descriptionId}
                badge={isEdit ? 'CONFIG' : protocol.toUpperCase()}
                closeLabel={t('live.close', { defaultValue: '关闭' })}
                onClose={onClose}
                closeDisabled={isSubmitting}
              />

              {/* ── 2. 协议切换分段控制器 (仅新增时展示) ── */}
              {!isEdit && (
                <div className="shrink-0 border-b border-[var(--border)]/60 bg-[var(--bg-secondary)]/25 px-5 py-3 sm:px-6">
                  <div className="flex items-center gap-1.5 rounded-2xl border border-[var(--border)]/80 bg-[var(--bg-secondary)]/60 p-1">
                    <button
                      type="button"
                      onClick={() => {
                        setProtocol('rtsp')
                        setMainUrl('')
                      }}
                      className={`flex-1 rounded-xl py-1.5 text-xs font-semibold transition-all ${
                        protocol === 'rtsp'
                          ? 'bg-[var(--bg-surface-solid)] text-[var(--text-primary)] shadow-xs'
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
                      className={`flex-1 rounded-xl py-1.5 text-xs font-semibold transition-all ${
                        protocol === 'gb28181'
                          ? 'bg-[var(--bg-surface-solid)] text-[var(--text-primary)] shadow-xs'
                          : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                      }`}
                    >
                      {t('protocol.gb28181Access', { defaultValue: '国标 GB/T 28181 接入' })}
                    </button>
                  </div>
                </div>
              )}

              {/* ── 3. 主体表单区 (卡片分组 + 滚动条) ── */}
              <form
                onSubmit={handleSubmit}
                className="flex min-h-0 flex-1 flex-col overflow-hidden"
              >
                <div className="modal-form-content space-y-4">
                  {/* 错误警告提示条 */}
                  {errorMsg && (
                    <div className="flex items-center gap-2.5 rounded-2xl border border-[var(--status-danger-border)] bg-[var(--status-danger-soft)] p-3.5 text-xs text-[var(--status-danger)]">
                      <AlertCircle className="h-4 w-4 shrink-0" />
                      <span className="leading-relaxed">{errorMsg}</span>
                    </div>
                  )}

                  {/* ── 分组 1: 基本设备身份 ── */}
                  <div className="space-y-3.5 rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 p-4.5">
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-bold tracking-tight text-[var(--text-primary)]">
                        {t('drawer.deviceObject', { defaultValue: '设备基本信息' })}
                      </span>
                      <span className="text-[11px] text-[var(--text-muted)]">ID & Location</span>
                    </div>

                    <div>
                      <label
                        htmlFor={nameFieldId}
                        className="modal-form-label flex items-center justify-between"
                      >
                        <span>{t('manage.name')}</span>
                        <span className="text-[10px] font-normal text-[var(--status-danger)]">
                          {t('manage.requiredTag', { defaultValue: '* 必填' })}
                        </span>
                      </label>
                      <input
                        id={nameFieldId}
                        type="text"
                        required
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder={t('manage.namePlaceholder')}
                        className="modal-form-field mt-1.5"
                      />
                    </div>

                    <div>
                      <label
                        htmlFor={remarkFieldId}
                        className="modal-form-label flex items-center justify-between"
                      >
                        <span>{t('manage.remark')}</span>
                        <span className="text-[10px] font-normal text-[var(--text-muted)]">
                          {t('manage.optionalTag', { defaultValue: '可选' })}
                        </span>
                      </label>
                      <input
                        id={remarkFieldId}
                        type="text"
                        value={remark}
                        onChange={(e) => setRemark(e.target.value)}
                        placeholder={t('manage.remarkPlaceholder')}
                        className="modal-form-field mt-1.5"
                      />
                    </div>
                  </div>

                  {/* ── 分组 2: 视频流接入地址与网络 ── */}
                  <div className="space-y-3.5 rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 p-4.5">
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-bold tracking-tight text-[var(--text-primary)]">
                        {t('drawer.streamConfiguration', { defaultValue: '视频流接入地址' })}
                      </span>
                      <span className="font-mono text-[11px] text-[var(--text-muted)]">
                        {protocol.toUpperCase()}
                      </span>
                    </div>

                    {/* GB28181 级联下拉 */}
                    {protocol === 'gb28181' ? (
                      <div className="space-y-3">
                        <div>
                          <label htmlFor={deviceFieldId} className="modal-form-label mb-1 block">
                            {t('protocol.selectGbDevice', { defaultValue: '选择注册的国标设备' })}
                          </label>
                          {gbDevices.length === 0 ? (
                            <p className="rounded-xl border border-dashed border-[var(--border)] p-3 text-xs leading-relaxed text-[var(--text-muted)]">
                              {t('protocol.noGbDevices', {
                                defaultValue:
                                  '当前暂无注册上线的国标设备。请先让 IPC / NVR 对接到本机 SIP 服务器。',
                              })}
                            </p>
                          ) : (
                            <div className="relative">
                              <select
                                id={deviceFieldId}
                                value={selectedGbDevice}
                                onChange={(e) => handleGbDeviceChange(e.target.value)}
                                className="modal-form-field appearance-none pr-9"
                              >
                                {gbDevices.map((d) => (
                                  <option key={d.deviceId} value={d.deviceId}>
                                    {d.name || d.deviceId} ({d.ipAddr})
                                  </option>
                                ))}
                              </select>
                              <ChevronDown className="pointer-events-none absolute top-1/2 right-3 h-4 w-4 -translate-y-1/2 text-[var(--text-muted)]" />
                            </div>
                          )}
                        </div>

                        {currentGbDev && currentGbDev.channels && (
                          <div>
                            <label htmlFor={channelFieldId} className="modal-form-label mb-1 block">
                              {t('protocol.selectGbChannel', { defaultValue: '选择视频通道' })}
                            </label>
                            <div className="relative">
                              <select
                                id={channelFieldId}
                                value={selectedGbChannel}
                                onChange={(e) => handleGbChannelChange(e.target.value)}
                                className="modal-form-field appearance-none pr-9"
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
                              <ChevronDown className="pointer-events-none absolute top-1/2 right-3 h-4 w-4 -translate-y-1/2 text-[var(--text-muted)]" />
                            </div>
                          </div>
                        )}
                      </div>
                    ) : null}

                    {/* 主码流地址 */}
                    <div>
                      <div className="flex items-center justify-between">
                        <label
                          htmlFor={mainUrlFieldId}
                          className="modal-form-label flex items-center gap-1.5"
                        >
                          <span>
                            {protocol === 'gb28181'
                              ? t('protocol.gbUri', { defaultValue: '国标 URI 地址' })
                              : t('manage.mainRtspUrl')}
                          </span>
                          <span className="text-[10px] font-normal text-[var(--status-danger)]">
                            {t('manage.requiredTag', { defaultValue: '* 必填' })}
                          </span>
                        </label>
                        {protocol === 'rtsp' && (
                          <div className="flex items-center gap-2">
                            <button
                              type="button"
                              onClick={() => setIsLanScanOpen(true)}
                              className="inline-flex items-center gap-1 text-[11px] font-medium text-emerald-600 hover:underline dark:text-emerald-400"
                            >
                              <Radio className="h-3 w-3" />
                              <span>{t('discovery.scanLan', { defaultValue: '局域网嗅探' })}</span>
                            </button>
                            {isDeducing && (
                              <span className="flex items-center gap-1 text-[10px] text-cyan-500">
                                <Loader2 className="h-3 w-3 animate-spin" />
                                <span>
                                  {t('discovery.deducing', { defaultValue: '推导中...' })}
                                </span>
                              </span>
                            )}
                          </div>
                        )}
                      </div>
                      <input
                        id={mainUrlFieldId}
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
                        className={`font-data modal-form-field mt-1.5 ${
                          protocol === 'gb28181' ? 'opacity-80' : ''
                        }`}
                      />
                    </div>

                    {/* 子码流 RTSP 地址 (仅 RTSP 模式支持) */}
                    {protocol === 'rtsp' && (
                      <div>
                        <div className="flex items-center justify-between">
                          <label
                            htmlFor={subUrlFieldId}
                            className="modal-form-label flex items-center gap-1.5"
                          >
                            <span>{t('manage.subRtspUrl')}</span>
                            <span className="text-[10px] font-normal text-[var(--text-muted)]">
                              {t('manage.optionalTag', { defaultValue: '可选' })}
                            </span>
                          </label>
                          {subCandidates.length > 0 && (
                            <span className="flex items-center gap-1 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">
                              <Sparkles className="h-3 w-3" />
                              <span>{t('manage.subCandidatesHint')}</span>
                            </span>
                          )}
                        </div>
                        <input
                          id={subUrlFieldId}
                          type="text"
                          value={subUrl}
                          onChange={(e) => setSubUrl(e.target.value)}
                          placeholder={t('manage.subRtspPlaceholder')}
                          className="font-data modal-form-field mt-1.5"
                        />
                        <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--text-muted)]">
                          {t('manage.subRtspHint')}
                        </p>

                        {/* 智能子码流候选芯片预设 */}
                        {subCandidates.length > 0 && (
                          <div className="mt-2.5 flex flex-wrap gap-1.5">
                            {subCandidates.map((c, i) => {
                              const isSelected = subUrl === c.subUrl
                              return (
                                <button
                                  type="button"
                                  key={i}
                                  onClick={() => setSubUrl(c.subUrl)}
                                  className={`font-data flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[11px] transition-all ${
                                    isSelected
                                      ? 'border border-emerald-500/50 bg-emerald-500/15 font-semibold text-emerald-700 dark:text-emerald-300'
                                      : 'border border-[var(--border)]/80 bg-[var(--bg-surface-solid)] text-[var(--text-secondary)] hover:border-[var(--accent)] hover:text-[var(--text-primary)]'
                                  }`}
                                  title={c.description}
                                >
                                  {isSelected && <Check className="h-3 w-3 text-emerald-500" />}
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
                  </div>

                  {/* ── 分组 3: AI 分析码流策略 ── */}
                  <div className="rounded-2xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 p-4.5">
                    <StreamModeSelector value={streamMode} onChange={setStreamMode} />
                  </div>
                </div>

                {/* ── 4. 现代化 SaaS 底部吸底操作栏 ── */}
                <div className="modal-form-footer">
                  <div className="modal-form-actions">
                    <button
                      type="button"
                      onClick={onClose}
                      disabled={isSubmitting}
                      className="modal-form-button modal-form-button--secondary"
                    >
                      {t('manage.cancel')}
                    </button>
                    <button
                      type="submit"
                      disabled={isSubmitting}
                      className="modal-form-button modal-form-button--primary"
                    >
                      {isSubmitting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
                      <span>{submitButtonText}</span>
                    </button>
                  </div>
                </div>
              </form>
            </motion.div>
          </div>
        )}
      </AnimatePresence>

      <LanDiscoveryModal
        isOpen={isLanScanOpen}
        onClose={() => setIsLanScanOpen(false)}
        onSelectDevice={handleSelectDiscovered}
      />
    </>
  )
}
