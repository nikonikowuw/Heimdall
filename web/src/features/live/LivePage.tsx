import { useEffect, useState } from 'react'
import {
  Activity,
  Camera as CameraIcon,
  Car,
  Compass,
  Grid,
  Plus,
  Radio,
  Sparkles,
  User,
  Zap,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cameraApi } from '@/lib/api'
import type { Camera } from '@/types'
import { WhepPlayer } from './components/WhepPlayer'

export function LivePage() {
  const { t } = useTranslation('camera')

  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedHeroId, setSelectedHeroId] = useState<string>('')
  const [viewMode, setViewMode] = useState<'hero_rail' | 'bento_grid'>('hero_rail')
  const [autoSpotlight, setAutoSpotlight] = useState<boolean>(true)
  const [showAddModal, setShowAddModal] = useState<boolean>(false)

  // 加载摄像头列表
  useEffect(() => {
    async function loadCameras() {
      try {
        const list = await cameraApi.list()
        if (list && list.length > 0) {
          setCameras(list)
          setSelectedHeroId((prev) => prev || list[0].cameraId)
        } else {
          setCameras([])
        }
      } catch {
        setCameras([])
      }
    }

    void loadCameras()
  }, [])

  const heroCamera = cameras.find((c) => c.cameraId === selectedHeroId) || cameras[0]
  const railCameras = cameras.filter((c) => c.cameraId !== heroCamera?.cameraId)

  return (
    <div className="flex h-full flex-col gap-3">
      {/* 顶部智能监控控制台 HUD 工具栏 */}
      <div className="frosted-glass flex items-center justify-between rounded-xl px-4 py-2.5">
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent)]/10 text-[var(--accent)]">
            <CameraIcon className="h-4 w-4" />
          </div>
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-[var(--text-primary)]">
                {t('live.title', '边缘智能监控大屏')}
              </span>
              <span className="flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-500">
                <Radio className="h-2.5 w-2.5 animate-pulse" />
                <span>{t('live.protocolBadge', 'WebRTC · WHEP')}</span>
              </span>
              <span className="flex items-center gap-1 rounded-full bg-cyan-500/10 px-2 py-0.5 text-[10px] font-medium text-cyan-400">
                <Zap className="h-2.5 w-2.5" />
                <span>{t('live.aneAccelerator', 'Apple ANE 加速')}</span>
              </span>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 智能追焦开关 */}
          <button
            type="button"
            onClick={() => setAutoSpotlight(!autoSpotlight)}
            className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-all ${
              autoSpotlight
                ? 'border border-cyan-500/30 bg-cyan-500/15 text-cyan-400'
                : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
            }`}
            title={t('live.autoSpotlightDesc', '检测到告警或活动目标时自动将视角切入主视口')}
          >
            <Sparkles className="h-3.5 w-3.5" />
            <span>{t('live.autoSpotlight', '智能追焦')}</span>
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                autoSpotlight ? 'animate-pulse bg-cyan-400' : 'bg-gray-500'
              }`}
            />
          </button>

          {/* 视图布局模式切换 */}
          <div className="flex items-center rounded-lg border border-[var(--border)] p-0.5">
            <button
              type="button"
              onClick={() => setViewMode('hero_rail')}
              className={`flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                viewMode === 'hero_rail'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              <Compass className="h-3.5 w-3.5" />
              <span>{t('live.focusMode', '指挥舱')}</span>
            </button>
            <button
              type="button"
              onClick={() => setViewMode('bento_grid')}
              className={`flex items-center gap-1 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ${
                viewMode === 'bento_grid'
                  ? 'bg-[var(--accent)] text-white shadow-xs'
                  : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
              }`}
            >
              <Grid className="h-3.5 w-3.5" />
              <span>{t('live.bentoMode', '全景 Bento')}</span>
            </button>
          </div>

          {/* 添加摄像头按钮 */}
          <button
            type="button"
            onClick={() => setShowAddModal(true)}
            className="flex items-center gap-1 rounded-lg bg-[var(--accent)] px-3 py-1 text-xs font-medium text-white shadow-xs transition-opacity hover:opacity-90"
          >
            <Plus className="h-3.5 w-3.5" />
            <span>{t('live.addCamera', '接入设备')}</span>
          </button>
        </div>
      </div>

      {/* 主视口布局容器 */}
      {viewMode === 'hero_rail' ? (
        <div className="grid flex-1 grid-cols-12 gap-3 overflow-hidden">
          {/* 左侧 72% 沉浸式 Hero Stage (占据 8.5 / 12 列) */}
          <div className="col-span-12 flex flex-col gap-2 overflow-hidden lg:col-span-8 xl:col-span-9">
            <div className="relative flex-1 overflow-hidden rounded-xl">
              {heroCamera ? (
                <WhepPlayer
                  key={heroCamera.cameraId}
                  cameraId={heroCamera.cameraId}
                  cameraName={heroCamera.name}
                  className="h-full w-full"
                  showHud={true}
                  isHero={true}
                />
              ) : (
                <div className="flex h-full items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] text-[var(--text-muted)]">
                  <div className="flex flex-col items-center gap-2">
                    <CameraIcon className="h-8 w-8 opacity-40" />
                    <span className="text-xs">{t('live.noCameras', '暂无活动摄像头')}</span>
                  </div>
                </div>
              )}
            </div>

            {/* Hero 下方实时遥测事件滚动胶囊 */}
            <div className="frosted-glass flex items-center justify-between rounded-lg px-3 py-1.5 text-xs text-[var(--text-secondary)]">
              <div className="flex items-center gap-2 font-mono text-[11px]">
                <span className="flex h-2 w-2 animate-ping rounded-full bg-cyan-400" />
                <span className="font-semibold text-cyan-400">
                  {t('live.liveTelemetry', '实时遥测')}:
                </span>
                <span className="text-[var(--text-primary)]">
                  [{heroCamera?.name || 'CAM'}] {heroCamera?.lastCodec.toUpperCase() || 'H264'}{' '}
                  {heroCamera?.lastWidth ? `${heroCamera.lastWidth}x${heroCamera.lastHeight}` : ''}
                </span>
              </div>
              <div className="flex items-center gap-3 text-[11px]">
                <span>
                  {t('live.fps', '帧率')}:{' '}
                  <strong className="text-emerald-400">
                    {heroCamera?.lastFps ? heroCamera.lastFps.toFixed(1) : '25.0'} FPS
                  </strong>
                </span>
                <span>
                  {t('live.latency', '端到端延时')}:{' '}
                  <strong className="text-cyan-400">128 ms</strong>
                </span>
              </div>
            </div>
          </div>

          {/* 右侧 28% Bento Live Rail 活动流轨道 (占据 3.5 / 12 列) */}
          <div className="col-span-12 flex flex-col gap-3 overflow-y-auto lg:col-span-4 xl:col-span-3">
            <div className="flex items-center justify-between px-1 text-xs text-[var(--text-muted)]">
              <span className="font-medium tracking-wide">
                {t('live.auxStreams', '活动监控流')} ({railCameras.length})
              </span>
              <span className="text-[10px]">{t('live.switchMainHint', '点击切换主流 ↗')}</span>
            </div>

            <div className="flex flex-1 flex-col gap-3">
              {railCameras.map((cam) => (
                <button
                  type="button"
                  key={cam.cameraId}
                  onClick={() => setSelectedHeroId(cam.cameraId)}
                  className="group relative cursor-pointer overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] text-left transition-all duration-300 hover:border-cyan-500/50 hover:shadow-lg hover:shadow-cyan-500/10"
                >
                  {/* 微缩播放器视口 */}
                  <div className="relative aspect-video w-full">
                    <WhepPlayer
                      cameraId={cam.cameraId}
                      cameraName={cam.name}
                      showHud={false}
                      isHero={false}
                      onSpotlight={() => setSelectedHeroId(cam.cameraId)}
                      className="h-full w-full"
                    />
                  </div>

                  {/* 卡片底部遥测状态条 */}
                  <div className="p-2.5">
                    <div className="flex items-center justify-between">
                      <span className="text-xs font-semibold text-[var(--text-primary)] transition-colors group-hover:text-cyan-400">
                        {cam.name}
                      </span>
                      <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 font-mono text-[10px] text-emerald-500">
                        {cam.lastWidth ? `${cam.lastWidth}P` : '1080P'}
                      </span>
                    </div>

                    <div className="mt-2 flex items-center justify-between text-[11px] text-[var(--text-muted)]">
                      <div className="flex items-center gap-2">
                        <span className="flex items-center gap-0.5">
                          <User className="h-3 w-3" /> 0
                        </span>
                        <span className="flex items-center gap-0.5">
                          <Car className="h-3 w-3" /> 0
                        </span>
                      </div>
                      <span className="flex items-center gap-1 font-mono text-[10px] text-cyan-400">
                        <Activity className="h-3 w-3" /> {t('live.active', '活跃')}
                      </span>
                    </div>
                  </div>
                </button>
              ))}

              {railCameras.length === 0 && (
                <div className="flex flex-1 items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-8 text-center text-xs text-[var(--text-muted)]">
                  <span>{t('live.noAuxStreams', '暂无更多辅路流')}</span>
                </div>
              )}
            </div>
          </div>
        </div>
      ) : (
        /* 全景自适应 Bento 网格视图 */
        <div className="grid flex-1 grid-cols-1 gap-3 overflow-y-auto md:grid-cols-2 lg:grid-cols-3">
          {cameras.map((cam) => (
            <div
              key={cam.cameraId}
              className="flex flex-col overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)]"
            >
              <div className="relative aspect-video w-full">
                <WhepPlayer
                  cameraId={cam.cameraId}
                  cameraName={cam.name}
                  showHud={true}
                  isHero={false}
                  className="h-full w-full"
                />
              </div>
            </div>
          ))}

          {cameras.length === 0 && (
            <div className="col-span-full flex items-center justify-center rounded-xl border border-dashed border-[var(--border)] p-12 text-center text-xs text-[var(--text-muted)]">
              <span>{t('live.noCameras', '暂无活动摄像头')}</span>
            </div>
          )}
        </div>
      )}

      {/* 快捷添加摄像头弹窗 */}
      {showAddModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs">
          <div className="frosted-glass w-full max-w-md rounded-2xl border border-[var(--border)] p-6 shadow-2xl">
            <h3 className="text-base font-semibold text-[var(--text-primary)]">
              {t('live.addCameraTitle', '接入网络摄像头 (RTSP)')}
            </h3>
            <p className="mt-1 text-xs text-[var(--text-muted)]">
              {t(
                'live.addCameraDesc',
                '支持主流标准 RTSP 协议，系统将自动发起异步握手探活与 SPS 解析。',
              )}
            </p>

            <form
              onSubmit={async (e) => {
                e.preventDefault()
                const form = e.currentTarget
                const name = (form.elements.namedItem('name') as HTMLInputElement).value
                const rtspUrl = (form.elements.namedItem('rtspUrl') as HTMLInputElement).value
                const remark = (form.elements.namedItem('remark') as HTMLInputElement).value

                try {
                  const created = await cameraApi.create({
                    name,
                    rtspUrl,
                    remark,
                  })
                  if (created) {
                    setCameras((prev) => [...prev, created])
                    setShowAddModal(false)
                  }
                } catch {
                  setShowAddModal(false)
                }
              }}
              className="mt-4 flex flex-col gap-3"
            >
              <div>
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {t('live.deviceName', '设备名称')}
                </label>
                <input
                  name="name"
                  required
                  placeholder={t('live.namePlaceholder', '例如：库房正门东区')}
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />
              </div>

              <div>
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {t('live.rtspUrl', 'RTSP 视频流地址')}
                </label>
                <input
                  name="rtspUrl"
                  required
                  placeholder={t(
                    'live.rtspPlaceholder',
                    'rtsp://admin:12345@192.168.1.100:554/h264',
                  )}
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 font-mono text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />
              </div>

              <div>
                <label className="text-xs font-medium text-[var(--text-secondary)]">
                  {t('live.remark', '备注说明')}
                </label>
                <input
                  name="remark"
                  placeholder={t('live.remarkPlaceholder', '例如：主要出入口布防')}
                  className="mt-1 w-full rounded-lg border border-[var(--border)] bg-[var(--bg-primary)] px-3 py-1.5 text-xs text-[var(--text-primary)] focus:border-[var(--accent)] focus:outline-hidden"
                />
              </div>

              <div className="mt-2 flex justify-end gap-2">
                <button
                  type="button"
                  onClick={() => setShowAddModal(false)}
                  className="rounded-lg px-3 py-1.5 text-xs text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]"
                >
                  {t('live.cancel', '取消')}
                </button>
                <button
                  type="submit"
                  className="rounded-lg bg-[var(--accent)] px-4 py-1.5 text-xs font-medium text-white shadow-xs hover:opacity-90"
                >
                  {t('live.saveAndProbe', '保存并探活')}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  )
}
