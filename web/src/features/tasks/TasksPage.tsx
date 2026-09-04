import React from 'react'
import { Plus, Sliders, ToggleLeft } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export const TasksPage: React.FC = () => {
  const { t } = useTranslation('task')

  return (
    <div className="flex h-full flex-col gap-4">
      <div className="frosted-glass flex items-center justify-between rounded-xl p-3">
        <div className="flex items-center gap-2">
          <Sliders className="h-5 w-5 text-[var(--accent)]" />
          <span className="font-semibold text-[var(--text-primary)]">{t('title')}</span>
        </div>

        <button className="flex items-center gap-1.5 rounded-lg bg-[var(--accent)] px-3 py-1.5 text-xs font-medium text-white hover:opacity-90">
          <Plus className="h-3.5 w-3.5" />
          {t('addCamera')}
        </button>
      </div>

      <div className="grid flex-1 grid-cols-3 gap-4">
        {/* 左侧摄像头列表 */}
        <div className="frosted-glass flex flex-col gap-2 rounded-xl p-3">
          <span className="text-xs font-semibold text-[var(--text-muted)] uppercase">
            {t('camerasSection')} (1)
          </span>
          <div className="flex items-center justify-between rounded-lg border border-[var(--accent)] bg-[var(--accent-soft)] p-3">
            <div className="flex flex-col gap-1">
              <span className="text-xs font-semibold text-[var(--text-primary)]">CAM-01</span>
              <span className="font-mono text-[10px] text-[var(--text-muted)]">
                rtsp://192.168.1.100:554/live
              </span>
            </div>
            <ToggleLeft className="h-5 w-5 cursor-pointer text-[var(--accent)]" />
          </div>
        </div>

        {/* 右侧交互式规则画布与配置面板 */}
        <div className="frosted-glass col-span-2 flex flex-col justify-between rounded-xl p-4">
          <div className="flex items-center justify-between border-b border-[var(--border)] pb-3">
            <div>
              <h3 className="text-sm font-semibold text-[var(--text-primary)]">
                {t('ruleConfigTitle')}
              </h3>
              <p className="text-xs text-[var(--text-muted)]">{t('ruleConfigDesc')}</p>
            </div>
            <div className="flex gap-2">
              <button className="rounded-lg border border-[var(--border)] px-3 py-1 text-xs text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]">
                {t('drawRoi')}
              </button>
              <button className="rounded-lg border border-[var(--border)] px-3 py-1 text-xs text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]">
                {t('drawLine')}
              </button>
            </div>
          </div>

          <div className="relative my-4 flex flex-1 items-center justify-center rounded-lg border border-dashed border-[var(--border-strong)] bg-[var(--bg-secondary)]">
            <span className="text-xs text-[var(--text-muted)]">{t('canvasPlaceholder')}</span>
          </div>

          <div className="flex justify-end gap-2 pt-2">
            <button className="rounded-lg bg-[var(--accent)] px-4 py-1.5 text-xs font-medium text-white hover:opacity-90">
              {t('saveRules')}
            </button>
          </div>
        </div>
      </div>
    </div>
  )
}
