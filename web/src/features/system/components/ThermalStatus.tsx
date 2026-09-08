/**
 * 温度状态显示组件
 *
 * 显示所有温度传感器的读数和降频状态。
 * 使用颜色编码表示温度级别，支持 CPU/NPU/DDR 等多种传感器。
 */

import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { Thermometer, AlertTriangle } from 'lucide-react'
import type { ThermalMetrics, ThermalZone } from '../../../types/system'
import { getTemperatureColor, getTemperatureBgColor } from './colors'

interface ThermalStatusProps {
  thermal: ThermalMetrics
  className?: string
}

function getTemperatureLabel(temp: number): string {
  if (temp >= 85) return 'Critical'
  if (temp >= 70) return 'Hot'
  if (temp >= 50) return 'Warm'
  return 'Normal'
}

function getTypeLabel(type: string): string {
  const labels: Record<string, string> = {
    cpu: 'CPU',
    npu: 'NPU',
    ddr: 'DDR',
    gpu: 'GPU',
    board: 'Board',
    other: 'Other',
  }
  return labels[type] || type
}

function ThermalZoneCard({ zone }: { zone: ThermalZone }) {
  const { t } = useTranslation('system')
  const color = getTemperatureColor(zone.temperature)
  const bgColor = getTemperatureBgColor(zone.temperature)

  const levelKey =
    zone.temperature >= 85
      ? 'critical'
      : zone.temperature >= 70
        ? 'hot'
        : zone.temperature >= 50
          ? 'warm'
          : 'normal'
  const label = t(`thermal.levels.${levelKey}`, {
    defaultValue: getTemperatureLabel(zone.temperature),
  })

  const typeLabel = t(`thermal.types.${zone.typeLabel}`, {
    defaultValue: getTypeLabel(zone.typeLabel),
  })

  return (
    <div
      className="flex items-center gap-3 rounded-xl border p-3 transition-all hover:scale-[1.01]"
      style={{
        borderColor: `color-mix(in srgb, ${color} 30%, transparent)`,
        backgroundColor: bgColor,
      }}
    >
      {/* 温度计图标 */}
      <div
        className="flex h-10 w-10 items-center justify-center rounded-xl"
        style={{ backgroundColor: `color-mix(in srgb, ${color} 15%, transparent)` }}
      >
        <Thermometer className="h-5 w-5" style={{ color }} />
      </div>

      {/* 温度信息 */}
      <div className="flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-[var(--text-primary)]">{typeLabel}</span>
          <span
            className="rounded-full px-2 py-0.5 text-[10px] font-medium"
            style={{
              backgroundColor: `color-mix(in srgb, ${color} 20%, transparent)`,
              color,
            }}
          >
            {label}
          </span>
        </div>
        <p className="text-[11px] text-[var(--text-muted)]">{zone.name}</p>
      </div>

      {/* 温度值 */}
      <div className="text-right">
        <p className="text-lg font-bold tabular-nums" style={{ color }}>
          {zone.temperature.toFixed(1)}°C
        </p>
      </div>
    </div>
  )
}

export function ThermalStatus({ thermal, className = '' }: ThermalStatusProps) {
  const { t } = useTranslation('system')

  // 按温度从高到低排序
  const sortedZones = useMemo(() => {
    return [...thermal.zones].sort((a, b) => b.temperature - a.temperature)
  }, [thermal.zones])

  // 获取最高温度
  const maxTemp = useMemo(() => {
    if (sortedZones.length === 0) return 0
    return sortedZones[0].temperature
  }, [sortedZones])

  if (sortedZones.length === 0) {
    return (
      <div className={`flex items-center justify-center text-[var(--text-muted)] ${className}`}>
        {t('thermal.noData', { defaultValue: 'No temperature data available' })}
      </div>
    )
  }

  return (
    <div className={`space-y-3 ${className}`}>
      {/* 降频警告 */}
      {thermal.throttleActive && (
        <div className="flex items-center gap-2 rounded-lg border border-[var(--accent-amber)]/30 bg-[var(--accent-amber)]/10 p-3">
          <AlertTriangle className="h-4 w-4 text-[var(--accent-amber)]" />
          <span className="text-[13px] font-medium text-[var(--accent-amber)]">
            {t('thermal.throttling', {
              defaultValue: 'Thermal throttling active - Performance may be reduced',
            })}
          </span>
        </div>
      )}

      {/* 温度汇总 */}
      <div className="flex items-center justify-between rounded-lg bg-[var(--bg-secondary)] p-3">
        <div className="flex items-center gap-2">
          <Thermometer className="h-4 w-4 text-[var(--text-muted)]" />
          <span className="text-[13px] text-[var(--text-secondary)]">
            {t('thermal.highest', { defaultValue: 'Highest temperature' })}
          </span>
        </div>
        <span
          className="text-lg font-bold tabular-nums"
          style={{ color: getTemperatureColor(maxTemp) }}
        >
          {maxTemp.toFixed(1)}°C
        </span>
      </div>

      {/* 温度传感器列表 */}
      <div className="space-y-2">
        {sortedZones.map((zone) => (
          <ThermalZoneCard key={zone.name} zone={zone} />
        ))}
      </div>
    </div>
  )
}
