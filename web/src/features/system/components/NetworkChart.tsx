/**
 * 网络流量统计组件
 *
 * 显示所有网络接口的流量统计，包括接收/发送字节数、实时传输速率、丢包/错误统计，
 * 以及多时间步实时速率曲线图（Sparkline）。
 */

import { useMemo, useRef, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Wifi, WifiOff, ArrowDown, ArrowUp, AlertTriangle, TrendingUp } from 'lucide-react'
import type { NetworkInterfaceMetrics } from '../../../types/system'

interface NetworkChartProps {
  interfaces: NetworkInterfaceMetrics[]
  className?: string
}

interface NetSpeed {
  rxSpeed: number
  txSpeed: number
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'

  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const k = 1024
  const i = Math.floor(Math.log(bytes) / Math.log(k))

  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${units[i]}`
}

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec <= 0) return '0 B/s'
  const units = ['B/s', 'KB/s', 'MB/s', 'GB/s']
  const k = 1024
  const i = Math.floor(Math.log(bytesPerSec) / Math.log(k))
  const idx = Math.min(Math.max(i, 0), units.length - 1)
  return `${(bytesPerSec / Math.pow(k, idx)).toFixed(1)} ${units[idx]}`
}

function formatPackets(packets: number): string {
  if (packets === 0) return '0'

  if (packets >= 1000000) {
    return `${(packets / 1000000).toFixed(1)}M`
  }
  if (packets >= 1000) {
    return `${(packets / 1000).toFixed(1)}K`
  }

  return packets.toString()
}

/** 实时流量趋势迷你折线图 */
function TrafficSparkline({ history, height = 38 }: { history: NetSpeed[]; height?: number }) {
  const { t } = useTranslation('system')

  if (history.length < 2) {
    return (
      <div className="flex h-[38px] w-full items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)]/50 text-[10px] text-[var(--text-muted)]">
        <TrendingUp className="mr-1 h-3 w-3 opacity-60" />
        {t('network.collectingTrend', { defaultValue: 'Collecting real-time curve...' })}
      </div>
    )
  }

  const maxSpeed = Math.max(...history.map((h) => Math.max(h.rxSpeed, h.txSpeed)), 1024)
  const width = 280
  const step = width / Math.max(history.length - 1, 1)

  const rxPoints = history.map((h, i) => {
    const x = i * step
    const y = height - (h.rxSpeed / maxSpeed) * (height - 6) - 3
    return `${x.toFixed(1)},${y.toFixed(1)}`
  })

  const txPoints = history.map((h, i) => {
    const x = i * step
    const y = height - (h.txSpeed / maxSpeed) * (height - 6) - 3
    return `${x.toFixed(1)},${y.toFixed(1)}`
  })

  return (
    <div className="relative h-[38px] w-full overflow-hidden rounded-lg border border-[var(--border)] bg-[var(--bg-secondary)]/30 px-1 py-0.5">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        className="h-full w-full overflow-visible"
        preserveAspectRatio="none"
      >
        {/* RX 实时速率曲线 (绿) */}
        <polyline
          fill="none"
          stroke="var(--accent-green)"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
          points={rxPoints.join(' ')}
        />
        {/* TX 实时速率曲线 (蓝) */}
        <polyline
          fill="none"
          stroke="var(--accent)"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeLinejoin="round"
          points={txPoints.join(' ')}
        />
      </svg>
    </div>
  )
}

function TrafficBar({
  rxBytes,
  txBytes,
  maxBytes,
  speed,
}: {
  rxBytes: number
  txBytes: number
  maxBytes: number
  speed?: NetSpeed
}) {
  const totalSpeed = (speed?.rxSpeed ?? 0) + (speed?.txSpeed ?? 0)
  let rxPercent: number
  let txPercent: number

  if (totalSpeed > 0) {
    rxPercent = ((speed?.rxSpeed ?? 0) / totalSpeed) * 100
    txPercent = ((speed?.txSpeed ?? 0) / totalSpeed) * 100
  } else {
    rxPercent = maxBytes > 0 ? (rxBytes / maxBytes) * 100 : 0
    txPercent = maxBytes > 0 ? (txBytes / maxBytes) * 100 : 0
  }

  return (
    <div className="flex h-2 w-full gap-1 overflow-hidden rounded-full bg-[var(--bg-secondary)]">
      {/* RX 流量 */}
      <div
        className="rounded-full bg-[var(--accent-green)] transition-all duration-700 ease-out"
        style={{ width: `${rxPercent}%` }}
      />
      {/* TX 流量 */}
      <div
        className="rounded-full bg-[var(--accent)] transition-all duration-700 ease-out"
        style={{ width: `${txPercent}%` }}
      />
    </div>
  )
}

function ErrorIndicator({ errors, dropped }: { errors: number; dropped: number }) {
  const { t } = useTranslation('system')

  if (errors === 0 && dropped === 0) {
    return null
  }

  return (
    <div className="flex items-center gap-1 text-[var(--accent-amber)]">
      <AlertTriangle className="h-3 w-3" />
      <span className="text-[10px]">
        {errors > 0 && `${errors} ${t('network.err', { defaultValue: 'err' })}`}
        {errors > 0 && dropped > 0 && ' / '}
        {dropped > 0 && `${dropped} ${t('network.drop', { defaultValue: 'drop' })}`}
      </span>
    </div>
  )
}

function InterfaceRow({
  iface,
  maxBytes,
  speed,
  history = [],
}: {
  iface: NetworkInterfaceMetrics
  maxBytes: number
  speed?: NetSpeed
  history?: NetSpeed[]
}) {
  const { t } = useTranslation('system')

  return (
    <div className="flex flex-col gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-3.5 transition-all hover:border-[var(--border-strong)]">
      {/* 接口名称和状态 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          {iface.linkUp ? (
            <Wifi className="h-3.5 w-3.5 text-[var(--accent-green)]" />
          ) : (
            <WifiOff className="h-3.5 w-3.5 text-[var(--text-muted)]" />
          )}
          <span className="text-[13px] font-medium text-[var(--text-primary)]">{iface.name}</span>
          {iface.speedMbps && (
            <span className="text-[11px] text-[var(--text-muted)]">{iface.speedMbps} Mbps</span>
          )}
        </div>

        <ErrorIndicator
          errors={iface.rxErrors + iface.txErrors}
          dropped={iface.rxDropped + iface.txDropped}
        />
      </div>

      {/* 实时趋势走势曲线 */}
      <TrafficSparkline history={history} />

      {/* 流量比例条 */}
      <TrafficBar
        rxBytes={iface.rxBytes}
        txBytes={iface.txBytes}
        maxBytes={maxBytes}
        speed={speed}
      />

      {/* 流量统计 */}
      <div className="grid grid-cols-2 gap-3 pt-0.5">
        {/* RX */}
        <div className="flex items-center gap-2">
          <ArrowDown className="h-3.5 w-3.5 shrink-0 text-[var(--accent-green)]" />
          <div className="min-w-0">
            <div className="flex items-baseline gap-1.5">
              <span className="text-[11px] text-[var(--text-muted)]">
                {t('network.rx', { defaultValue: 'RX (Receive)' })}
              </span>
              {speed && speed.rxSpeed > 0 && (
                <span className="text-xs font-bold text-[var(--accent-green)] tabular-nums">
                  {formatSpeed(speed.rxSpeed)}
                </span>
              )}
            </div>
            <p className="truncate text-xs font-bold text-[var(--text-primary)] tabular-nums">
              {formatBytes(iface.rxBytes)}
            </p>
            <p className="text-[10px] text-[var(--text-muted)] tabular-nums">
              {formatPackets(iface.rxPackets)} {t('network.pkts', { defaultValue: 'pkts' })}
            </p>
          </div>
        </div>

        {/* TX */}
        <div className="flex items-center gap-2">
          <ArrowUp className="h-3.5 w-3.5 shrink-0 text-[var(--accent)]" />
          <div className="min-w-0">
            <div className="flex items-baseline gap-1.5">
              <span className="text-[11px] text-[var(--text-muted)]">
                {t('network.tx', { defaultValue: 'TX (Send)' })}
              </span>
              {speed && speed.txSpeed > 0 && (
                <span className="text-xs font-bold text-[var(--accent)] tabular-nums">
                  {formatSpeed(speed.txSpeed)}
                </span>
              )}
            </div>
            <p className="truncate text-xs font-bold text-[var(--text-primary)] tabular-nums">
              {formatBytes(iface.txBytes)}
            </p>
            <p className="text-[10px] text-[var(--text-muted)] tabular-nums">
              {formatPackets(iface.txPackets)} {t('network.pkts', { defaultValue: 'pkts' })}
            </p>
          </div>
        </div>
      </div>
    </div>
  )
}

export function NetworkChart({ interfaces, className = '' }: NetworkChartProps) {
  const { t } = useTranslation('system')

  // 计算实时传输速率 (Bytes/s)
  const prevSamplesRef = useRef<
    Map<string, { rxBytes: number; txBytes: number; timestamp: number }>
  >(new Map())
  const [speeds, setSpeeds] = useState<Map<string, NetSpeed>>(new Map())
  const [speedHistories, setSpeedHistories] = useState<Map<string, NetSpeed[]>>(new Map())

  // 在 effect 中计算速率与有界历史，用于绘制走势曲线
  useEffect(() => {
    const now = Date.now()
    const prevMap = prevSamplesRef.current
    const nextSpeeds = new Map<string, NetSpeed>()

    for (const iface of interfaces) {
      const prev = prevMap.get(iface.name)
      if (prev && now > prev.timestamp) {
        const dt = (now - prev.timestamp) / 1000
        if (dt > 0.3) {
          const rxDelta = iface.rxBytes >= prev.rxBytes ? iface.rxBytes - prev.rxBytes : 0
          const txDelta = iface.txBytes >= prev.txBytes ? iface.txBytes - prev.txBytes : 0
          nextSpeeds.set(iface.name, {
            rxSpeed: rxDelta / dt,
            txSpeed: txDelta / dt,
          })
        }
      }
      prevMap.set(iface.name, {
        rxBytes: iface.rxBytes,
        txBytes: iface.txBytes,
        timestamp: now,
      })
    }

    setSpeeds(nextSpeeds)

    // 更新最近 15 次采样的历史窗口
    if (nextSpeeds.size > 0) {
      setSpeedHistories((prevHistories) => {
        const next = new Map(prevHistories)
        for (const [name, currentSpeed] of nextSpeeds.entries()) {
          const list = next.get(name) ? [...next.get(name)!] : []
          list.push(currentSpeed)
          if (list.length > 15) {
            list.shift()
          }
          next.set(name, list)
        }
        return next
      })
    }
  }, [interfaces])

  // 计算最大流量用于缩放进度条
  const maxBytes = useMemo(() => {
    if (interfaces.length === 0) return 1
    return Math.max(...interfaces.map((i) => Math.max(i.rxBytes, i.txBytes)), 1)
  }, [interfaces])

  if (interfaces.length === 0) {
    return (
      <div className={`flex items-center justify-center text-[var(--text-muted)] ${className}`}>
        {t('network.noData', { defaultValue: 'No network interfaces available' })}
      </div>
    )
  }

  return (
    <div className={`space-y-3 ${className}`}>
      {/* 图例 */}
      <div className="flex items-center justify-between text-[11px]">
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-1.5">
            <div className="h-2 w-2 rounded-full bg-[var(--accent-green)]" />
            <span className="text-[var(--text-muted)]">
              {t('network.rx', { defaultValue: 'RX (Receive)' })}
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <div className="h-2 w-2 rounded-full bg-[var(--accent)]" />
            <span className="text-[var(--text-muted)]">
              {t('network.tx', { defaultValue: 'TX (Send)' })}
            </span>
          </div>
        </div>
        <span className="text-[10px] text-[var(--text-muted)]">
          {t('network.trendWindow', { defaultValue: 'Live 30s Trend' })}
        </span>
      </div>

      {/* 接口列表 */}
      <div className="grid gap-3 sm:grid-cols-2">
        {interfaces.map((iface, idx) => (
          <InterfaceRow
            key={`${iface.name}-${idx}`}
            iface={iface}
            maxBytes={maxBytes}
            speed={speeds.get(iface.name)}
            history={speedHistories.get(iface.name)}
          />
        ))}
      </div>
    </div>
  )
}
