import { useState, useEffect } from 'react'
import { motion, AnimatePresence } from 'motion/react'
import { useTranslation } from 'react-i18next'
import {
  Radio,
  Copy,
  Check,
  Eye,
  EyeOff,
  RefreshCw,
  Server,
  FileText,
  CheckCircle2,
  X,
} from 'lucide-react'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { gb28181Api } from '@/lib/api'
import type { Gb28181ConfigResponse } from '@/types'

export function Gb28181Settings(): React.ReactElement {
  const { t } = useTranslation('system')
  const { t: tc } = useTranslation('common')
  const [data, setData] = useState<Gb28181ConfigResponse | null>(null)
  const [loading, setLoading] = useState(true)
  const [saving, setSaving] = useState(false)
  const [showPassword, setShowPassword] = useState(false)
  const [copiedField, setCopiedField] = useState<string | null>(null)
  const [showCardModal, setShowCardModal] = useState(false)
  const [cardCopied, setCardCopied] = useState(false)

  useDismissStack(showCardModal, () => setShowCardModal(false))

  // Form state
  const [sipId, setSipId] = useState('')
  const [sipDomain, setSipDomain] = useState('')
  const [sipPort, setSipPort] = useState(5060)
  const [sipPassword, setSipPassword] = useState('')
  const [rtpStart, setRtpStart] = useState(30000)
  const [rtpEnd, setRtpEnd] = useState(30500)
  const [autoSync, setAutoSync] = useState(true)
  const [heartbeatTimeout, setHeartbeatTimeout] = useState(180)

  const fetchConfig = async () => {
    setLoading(true)
    try {
      const res = await gb28181Api.getConfig()
      setData(res)
      setSipId(res.config.sipId)
      setSipDomain(res.config.sipDomain)
      setSipPort(res.config.sipPort)
      setSipPassword(res.config.sipPassword)
      setRtpStart(res.config.rtpPortRangeStart)
      setRtpEnd(res.config.rtpPortRangeEnd)
      setAutoSync(res.config.autoCatalogSync)
      setHeartbeatTimeout(res.config.heartbeatTimeoutSec)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }

  useEffect(() => {
    fetchConfig()
  }, [])

  const copyToClipboard = (text: string, fieldName: string) => {
    navigator.clipboard.writeText(text)
    setCopiedField(fieldName)
    setTimeout(() => setCopiedField(null), 2000)
  }

  const handleSave = async () => {
    setSaving(true)
    try {
      const updated = await gb28181Api.updateConfig({
        sipId,
        sipDomain,
        sipPort,
        sipPassword,
        rtpPortRangeStart: rtpStart,
        rtpPortRangeEnd: rtpEnd,
        autoCatalogSync: autoSync,
        heartbeatTimeoutSec: heartbeatTimeout,
      })
      if (data) {
        setData({
          ...data,
          config: updated,
        })
      }
    } catch {
      // ignore
    } finally {
      setSaving(false)
    }
  }

  const generateIpcCardText = () => {
    const hostIp = window.location.hostname
    return [
      '==========================================',
      `     ${t('gb28181.cardHeaderTitle', { defaultValue: 'Heimdall GB28181 摄像机对接指导卡' })}`,
      '==========================================',
      `${t('gb28181.cardServerIp', { defaultValue: 'SIP 服务器 IP' })}:    ${hostIp}`,
      `${t('gb28181.cardServerPort', { defaultValue: 'SIP 服务器端口' })}:   ${sipPort}`,
      `${t('gb28181.cardServerId', { defaultValue: 'SIP 服务器编码' })}:   ${sipId}`,
      `${t('gb28181.cardServerDomain', { defaultValue: 'SIP 服务器域' })}:     ${sipDomain}`,
      `${t('gb28181.cardPassword', { defaultValue: '接入鉴权密码' })}:     ${sipPassword}`,
      `${t('gb28181.cardHeartbeat', { timeout: heartbeatTimeout, defaultValue: `心跳间隔建议: 60 秒 (超时时间: ${heartbeatTimeout} 秒)` })}`,
      `${t('gb28181.cardStreamMode', { defaultValue: '流传输模式建议: TCP 被动 (推荐) 或 UDP' })}`,
      '==========================================',
    ].join('\n')
  }

  const handleCopyCard = () => {
    navigator.clipboard.writeText(generateIpcCardText())
    setCardCopied(true)
    setTimeout(() => setCardCopied(false), 2000)
  }

  if (loading && !data) {
    return (
      <div className="space-y-6">
        <div className="frosted-glass h-24 animate-pulse rounded-2xl border border-[var(--border)]" />
        <div className="frosted-glass h-48 animate-pulse rounded-2xl border border-[var(--border)]" />
      </div>
    )
  }

  const health = data?.health

  return (
    <div className="space-y-6">
      {/* 1. 服务运行状态横幅 */}
      <div className="frosted-glass flex flex-wrap items-center justify-between gap-4 rounded-2xl border border-[var(--border)] p-5">
        <div className="flex items-center gap-3.5">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-500">
            <Radio className="h-5 w-5" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-[var(--text-primary)]">
                {t('gb28181.serviceTitle', { defaultValue: '国标 GB/T 28181 原生服务' })}
              </span>
              <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2.5 py-0.5 text-xs font-medium text-emerald-500">
                <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
                {t('gb28181.statusOnline', { defaultValue: '运行中' })}
              </span>
            </div>
            <p className="mt-0.5 text-xs text-[var(--text-secondary)]">
              {t('gb28181.serviceDesc', {
                defaultValue:
                  '单二进制内嵌 SIP UAS 注册服务与 MPEG-PS 流式解复用引擎，无需外部流媒体中间件',
              })}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-6 text-xs text-[var(--text-secondary)]">
          <div>
            <span className="text-[var(--text-muted)]">
              {t('gb28181.listenPort', { defaultValue: '监听端口' })}:{' '}
            </span>
            <span className="font-mono font-medium text-[var(--text-primary)]">
              {health?.sipPort || sipPort}/UDP+TCP
            </span>
          </div>
          <div>
            <span className="text-[var(--text-muted)]">
              {t('gb28181.activeDevices', { defaultValue: '已握手设备' })}:{' '}
            </span>
            <span className="font-mono font-medium text-[var(--text-primary)]">
              {health?.onlineDevicesCount ?? 0}
            </span>
          </div>
          <button
            onClick={fetchConfig}
            className="flex items-center gap-1 rounded-lg border border-[var(--border)] px-2.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--surface-hover)]"
          >
            <RefreshCw className="h-3.5 w-3.5" />
            {tc('actions.refresh')}
          </button>
        </div>
      </div>

      {/* 2. 本机 SIP 平台参数 */}
      <div className="frosted-glass rounded-2xl border border-[var(--border)] p-5">
        <div className="mb-4 flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Server className="h-4 w-4 text-[var(--accent)]" />
            <h3 className="text-sm font-semibold text-[var(--text-primary)]">
              {t('gb28181.platformParams', {
                defaultValue: '本机 SIP 平台参数 (供摄像机后台填写)',
              })}
            </h3>
          </div>
          <button
            onClick={() => setShowCardModal(true)}
            className="flex items-center gap-1.5 text-xs font-medium text-[var(--accent)] hover:underline"
          >
            <FileText className="h-3.5 w-3.5" />
            {t('gb28181.exportCard', { defaultValue: '查看 IPC 对接指导卡' })}
          </button>
        </div>

        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          {/* SIP ID */}
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.sipId', { defaultValue: '平台国标编码 (SIP ID)' })}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={sipId}
                onChange={(e) => setSipId(e.target.value)}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
              />
              <button
                onClick={() => copyToClipboard(sipId, 'sipId')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--surface)] text-[var(--text-secondary)] hover:bg-[var(--surface-hover)]"
                title={tc('actions.copy')}
              >
                {copiedField === 'sipId' ? (
                  <Check className="h-4 w-4 text-emerald-500" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>

          {/* Domain */}
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.sipDomain', { defaultValue: 'SIP 服务器域 (Domain)' })}
            </label>
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={sipDomain}
                onChange={(e) => setSipDomain(e.target.value)}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
              />
              <button
                onClick={() => copyToClipboard(sipDomain, 'sipDomain')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--surface)] text-[var(--text-secondary)] hover:bg-[var(--surface-hover)]"
                title={tc('actions.copy')}
              >
                {copiedField === 'sipDomain' ? (
                  <Check className="h-4 w-4 text-emerald-500" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>

          {/* Port */}
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.sipPort', { defaultValue: 'SIP 监听端口 (Port)' })}
            </label>
            <input
              type="number"
              value={sipPort}
              onChange={(e) => setSipPort(Number(e.target.value))}
              className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
            />
          </div>

          {/* Password */}
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.password', { defaultValue: '设备接入密码 (Password)' })}
            </label>
            <div className="flex items-center gap-2">
              <div className="relative w-full">
                <input
                  type={showPassword ? 'text' : 'password'}
                  value={sipPassword}
                  onChange={(e) => setSipPassword(e.target.value)}
                  className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 pr-9 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
                />
                <button
                  type="button"
                  onClick={() => setShowPassword(!showPassword)}
                  className="absolute top-1/2 right-2.5 -translate-y-1/2 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                >
                  {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
              </div>
              <button
                onClick={() => copyToClipboard(sipPassword, 'password')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--surface)] text-[var(--text-secondary)] hover:bg-[var(--surface-hover)]"
                title={tc('actions.copy')}
              >
                {copiedField === 'password' ? (
                  <Check className="h-4 w-4 text-emerald-500" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </button>
            </div>
          </div>

          {/* Heartbeat Timeout */}
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.heartbeatTimeout', { defaultValue: '心跳超时判定时长 (秒)' })}
            </label>
            <input
              type="number"
              value={heartbeatTimeout}
              onChange={(e) => setHeartbeatTimeout(Number(e.target.value))}
              className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
            />
          </div>
        </div>
      </div>

      {/* 3. 媒体流端口池设置 */}
      <div className="frosted-glass rounded-2xl border border-[var(--border)] p-5">
        <h3 className="mb-3 text-sm font-semibold text-[var(--text-primary)]">
          {t('gb28181.portPoolTitle', { defaultValue: '媒体流端口池 (RTP Media Port Pool)' })}
        </h3>
        <p className="mb-4 text-xs text-[var(--text-secondary)]">
          {t('gb28181.portPoolDesc', {
            defaultValue:
              '点播推流时动态分配的一对一接收端口（偶数 RTP / 奇数 RTCP），任务结束由 RAII 租约自动回收',
          })}
        </p>

        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.portStart', { defaultValue: '端口池起始' })}
            </label>
            <input
              type="number"
              value={rtpStart}
              onChange={(e) => setRtpStart(Number(e.target.value))}
              className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-[var(--text-secondary)]">
              {t('gb28181.portEnd', { defaultValue: '端口池结束' })}
            </label>
            <input
              type="number"
              value={rtpEnd}
              onChange={(e) => setRtpEnd(Number(e.target.value))}
              className="w-full rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 py-2 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-none"
            />
          </div>
        </div>
        <p className="mt-2 text-right text-[11px] text-[var(--text-muted)]">
          {t('gb28181.maxStreams', { defaultValue: '最大支持并发流' })}:{' '}
          {Math.max(0, Math.floor((rtpEnd - rtpStart) / 2))}{' '}
          {t('gb28181.streams', { defaultValue: '路' })}
        </p>
      </div>

      {/* 4. 自动化策略 */}
      <div className="frosted-glass rounded-2xl border border-[var(--border)] p-5">
        <h3 className="mb-3 text-sm font-semibold text-[var(--text-primary)]">
          {t('gb28181.automationTitle', { defaultValue: '自动化策略' })}
        </h3>
        <label className="flex cursor-pointer items-center gap-3">
          <input
            type="checkbox"
            checked={autoSync}
            onChange={(e) => setAutoSync(e.target.checked)}
            className="h-4 w-4 rounded text-[var(--accent)] focus:ring-0"
          />
          <span className="text-xs text-[var(--text-primary)]">
            {t('gb28181.autoCatalogSync', {
              defaultValue: '设备首次注册成功后，自动下发 Catalog 目录扫描同步多通道',
            })}
          </span>
        </label>
      </div>

      {/* 底部保存动作 */}
      <div className="flex justify-end gap-3">
        <button
          onClick={handleSave}
          disabled={saving}
          className="flex items-center gap-2 rounded-xl bg-[var(--accent)] px-5 py-2.5 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
        >
          {saving ? (
            <RefreshCw className="h-4 w-4 animate-spin" />
          ) : (
            <CheckCircle2 className="h-4 w-4" />
          )}
          {tc('actions.save')}
        </button>
      </div>

      {/* 对接指导卡模态弹窗 */}
      <AnimatePresence>
        {showCardModal && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4 backdrop-blur-xs">
            <motion.div
              initial={{ scale: 0.95, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              exit={{ scale: 0.95, opacity: 0 }}
              className="frosted-glass w-full max-w-lg rounded-2xl border border-[var(--border)] bg-[var(--surface-elevated)] p-6 shadow-2xl"
            >
              <div className="mb-4 flex items-center justify-between">
                <div className="flex items-center gap-2.5">
                  <FileText className="h-5 w-5 text-[var(--accent)]" />
                  <h3 className="text-sm font-semibold text-[var(--text-primary)]">
                    {t('gb28181.cardTitle', { defaultValue: 'IPC / NVR 现场对接指导卡' })}
                  </h3>
                </div>
                <button
                  onClick={() => setShowCardModal(false)}
                  className="rounded-lg p-1 text-[var(--text-muted)] hover:bg-[var(--surface-hover)] hover:text-[var(--text-primary)]"
                >
                  <X className="h-4 w-4" />
                </button>
              </div>

              <p className="mb-3 text-xs text-[var(--text-secondary)]">
                {t('gb28181.cardDesc', {
                  defaultValue:
                    '登录海康、大华、宇视等摄像机 Web 管理后台，进入「网络」-「高级配置」-「平台接入」选 28181 填写：',
                })}
              </p>

              <pre className="mb-5 overflow-x-auto rounded-xl border border-[var(--border)] bg-black/40 p-4 font-mono text-[11px] leading-relaxed text-[var(--text-secondary)]">
                {generateIpcCardText()}
              </pre>

              <div className="flex justify-end gap-3">
                <button
                  onClick={() => setShowCardModal(false)}
                  className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] hover:bg-[var(--surface-hover)]"
                >
                  {t('cancel', { defaultValue: '关闭' })}
                </button>
                <button
                  onClick={handleCopyCard}
                  className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-2 text-xs font-medium text-white hover:opacity-90"
                >
                  {cardCopied ? (
                    <Check className="h-4 w-4 text-emerald-400" />
                  ) : (
                    <Copy className="h-4 w-4" />
                  )}
                  {cardCopied
                    ? t('saved', { defaultValue: '已复制全量参数' })
                    : t('gb28181.copyAll', { defaultValue: '复制全量参数文本' })}
                </button>
              </div>
            </motion.div>
          </div>
        )}
      </AnimatePresence>
    </div>
  )
}
