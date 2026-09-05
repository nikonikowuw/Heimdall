import React, { useEffect, useState } from 'react'
import { Layers, RefreshCw, ShieldAlert, ShieldCheck, Sliders, Video } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cameraApi, taskApi } from '../../lib/api'
import type { Camera, TaskConfigDto } from '../../types'
import { LiveRulesStudio } from './components/LiveRulesStudio'

interface TaskCameraCardProps {
  camera: Camera
  config?: TaskConfigDto
  onToggleArm: () => void
  onConfigure: () => void
  t: (key: string, options?: Record<string, unknown>) => string
}

function TaskCameraCard({
  camera,
  config,
  onToggleArm,
  onConfigure,
  t,
}: TaskCameraCardProps): React.ReactElement {
  const isArmed = config?.desiredEnabled ?? false
  const rulesCount = config?.rules?.length ?? 0
  const isMotionGateEco = config?.motionGate?.enabled ?? false

  return (
    <div
      className={`group flex flex-col space-y-3.5 rounded-2xl border bg-[var(--bg-surface)] p-4 shadow-xs transition-all duration-200 hover:shadow-md ${
        isArmed
          ? 'border-[var(--accent)]/40 hover:border-[var(--accent)]'
          : 'border-[var(--border)]'
      }`}
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div
            className={`flex h-8 w-8 items-center justify-center rounded-lg font-mono text-xs font-bold ${
              isArmed
                ? 'border border-rose-500/30 bg-rose-500/15 text-rose-500'
                : 'bg-slate-500/10 text-slate-500'
            }`}
          >
            <Video className="h-4 w-4" />
          </div>
          <div>
            <h4 className="text-xs font-semibold text-[var(--text-primary)]">{camera.name}</h4>
            <span className="font-mono text-[10px] text-[var(--text-muted)]">
              {camera.cameraId}
            </span>
          </div>
        </div>

        <button
          onClick={onToggleArm}
          className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-semibold transition-all ${
            isArmed
              ? 'border border-rose-500/30 bg-rose-500/15 text-rose-500'
              : 'bg-slate-500/10 text-slate-500 hover:bg-slate-500/20'
          }`}
        >
          {isArmed ? (
            <>
              <ShieldAlert className="h-3.5 w-3.5" />
              <span>{t('status.armed')}</span>
            </>
          ) : (
            <>
              <ShieldCheck className="h-3.5 w-3.5" />
              <span>{t('status.disarmed')}</span>
            </>
          )}
        </button>
      </div>

      <div className="space-y-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-2.5 font-mono text-[11px]">
        <div className="flex justify-between text-[var(--text-secondary)]">
          <span>{t('card.rtspDirect')}</span>
          <span className="max-w-[180px] truncate text-[var(--text-muted)]">{camera.rtspUrl}</span>
        </div>
        <div className="flex justify-between text-[var(--text-secondary)]">
          <span>{t('card.geometryRules')}</span>
          <span className="font-semibold text-[var(--accent)]">
            {t('card.rulesCount', { count: rulesCount })}
          </span>
        </div>
        <div className="flex justify-between text-[var(--text-secondary)]">
          <span>{t('card.motionGate')}</span>
          <span className="text-emerald-500">
            {isMotionGateEco ? t('card.motionGateEco') : t('card.motionGateAlways')}
          </span>
        </div>
      </div>

      <div className="flex items-center justify-between pt-1">
        <span className="font-mono text-[10px] text-[var(--text-muted)]">
          {camera.lastCodec?.toUpperCase() || 'H264'} ·{' '}
          {camera.lastWidth ? `${camera.lastWidth}x${camera.lastHeight}` : '1080P'}
        </span>
        <button
          onClick={onConfigure}
          className="flex items-center gap-1 rounded-lg bg-[var(--accent-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--accent)] shadow-2xs transition-all hover:bg-[var(--accent)] hover:text-white"
        >
          <Layers className="h-3.5 w-3.5" />
          <span>{t('actions.configureRules')}</span>
        </button>
      </div>
    </div>
  )
}

export function TasksPage(): React.ReactElement {
  const { t } = useTranslation('task')
  const { t: tc } = useTranslation('common')

  const [cameras, setCameras] = useState<Camera[]>([])
  const [taskConfigs, setTaskConfigs] = useState<Record<string, TaskConfigDto>>({})
  const [selectedCameraForConfig, setSelectedCameraForConfig] = useState<Camera | null>(null)
  const [isLoading, setIsLoading] = useState(false)

  const loadData = async (): Promise<void> => {
    setIsLoading(true)
    try {
      const [cams, tasks] = await Promise.all([cameraApi.list(), taskApi.list()])
      setCameras(cams)

      const configs: Record<string, TaskConfigDto> = {}
      for (const item of tasks) {
        configs[item.cameraId] = {
          cameraId: item.cameraId,
          name: item.name,
          desiredEnabled: item.desiredEnabled,
          rules: item.rules || [],
          motionGate: item.motionGate,
        }
      }
      setTaskConfigs(configs)
    } catch {
      // 优雅降级处理
    } finally {
      setIsLoading(false)
    }
  }

  useEffect(() => {
    loadData()
  }, [])

  const handleToggleArm = async (camera: Camera): Promise<void> => {
    const currentCfg = taskConfigs[camera.cameraId]
    const nextDesired = !(currentCfg?.desiredEnabled ?? false)

    try {
      const payload: TaskConfigDto = {
        cameraId: camera.cameraId,
        name: currentCfg?.name || `Task-${camera.cameraId}`,
        desiredEnabled: nextDesired,
        rules: currentCfg?.rules || [],
        motionGate: currentCfg?.motionGate || { enabled: true },
      }
      const updated = await taskApi.updateTask(camera.cameraId, payload)
      setTaskConfigs((prev) => ({ ...prev, [camera.cameraId]: updated }))
    } catch {
      // ignore
    }
  }

  if (selectedCameraForConfig) {
    return (
      <LiveRulesStudio
        initialCamera={selectedCameraForConfig}
        onBack={() => {
          setSelectedCameraForConfig(null)
          loadData()
        }}
      />
    )
  }

  const totalArmed = Object.values(taskConfigs).filter((cfg) => cfg.desiredEnabled).length

  return (
    <div className="flex h-full flex-col gap-4 bg-[var(--bg-primary)] p-4 text-[var(--text-primary)] select-none">
      {/* 顶部状态与操作栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-2xl p-3.5 shadow-xs">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <Sliders className="h-5 w-5" />
          </div>
          <div>
            <h2 className="text-sm font-semibold text-[var(--text-primary)]">{t('title')}</h2>
            <p className="text-xs text-[var(--text-muted)]">
              {t('stats.summary', { total: cameras.length, armed: totalArmed })}
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={loadData}
            disabled={isLoading}
            className="flex items-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-all hover:bg-[var(--accent-soft)] hover:text-[var(--accent)] disabled:opacity-50"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${isLoading ? 'animate-spin' : ''}`} />
            <span>{tc('actions.refresh')}</span>
          </button>
          {cameras.length > 0 && (
            <button
              onClick={() => setSelectedCameraForConfig(cameras[0])}
              className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-3.5 py-1.5 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90"
            >
              <Layers className="h-3.5 w-3.5" />
              <span>{t('actions.enterStudio')}</span>
            </button>
          )}
        </div>
      </div>

      {/* 摄像头通道布防卡片矩阵 */}
      <div className="frosted-glass flex-1 overflow-auto rounded-2xl p-4 shadow-xs">
        {cameras.length === 0 ? (
          <div className="py-24 text-center text-[var(--text-muted)]">
            <Video className="mx-auto mb-2 h-8 w-8 opacity-40" />
            <p className="font-medium text-[var(--text-secondary)]">{t('empty.noCameras')}</p>
            <p className="text-xs opacity-75">{t('empty.addCameraHint')}</p>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
            {cameras.map((camera) => (
              <TaskCameraCard
                key={camera.id}
                camera={camera}
                config={taskConfigs[camera.cameraId]}
                onToggleArm={() => handleToggleArm(camera)}
                onConfigure={() => setSelectedCameraForConfig(camera)}
                t={t}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  )
}
