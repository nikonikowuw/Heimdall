import React, { useState } from 'react'
import { AlertCircle, Camera, FileText, Globe, Moon, Sliders, Sun, Video } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { AlarmsPage } from '../features/alarms/AlarmsPage'
import { LivePage } from '../features/live/LivePage'
import { OplogPage } from '../features/oplog/OplogPage'
import { TasksPage } from '../features/tasks/TasksPage'
import { useLocale } from '../hooks/use-locale'
import type { Locale } from '../i18n'

export type NavTab = 'live' | 'tasks' | 'alarms' | 'oplog'

export const Layout: React.FC = () => {
  const { t } = useTranslation('common')
  const { locale, setLocale, supportedLocales } = useLocale()
  const [currentTab, setCurrentTab] = useState<NavTab>('live')
  const [isDark, setIsDark] = useState<boolean>(false)

  const toggleTheme = () => {
    const next = !isDark
    setIsDark(next)
    if (next) {
      document.documentElement.classList.add('dark')
    } else {
      document.documentElement.classList.remove('dark')
    }
  }

  const cycleLocale = () => {
    const currentIndex = supportedLocales.indexOf(locale)
    const nextLocale: Locale = supportedLocales[(currentIndex + 1) % supportedLocales.length]
    setLocale(nextLocale)
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-[var(--bg-primary)] font-sans antialiased">
      {/* 左侧紧凑工具导航栏 */}
      <aside className="frosted-glass flex w-16 flex-col items-center justify-between border-r border-[var(--border)] py-4">
        <div className="flex flex-col items-center gap-6">
          {/* Logo 标志 */}
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent)] text-white shadow-md">
            <Video className="h-5 w-5" />
          </div>

          {/* 导航菜单 */}
          <nav className="flex flex-col gap-3">
            <button
              onClick={() => setCurrentTab('live')}
              title={t('nav.live')}
              className={`flex h-10 w-10 items-center justify-center rounded-xl transition-colors ${
                currentTab === 'live'
                  ? 'bg-[var(--accent)] text-white'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
              }`}
            >
              <Camera className="h-5 w-5" />
            </button>
            <button
              onClick={() => setCurrentTab('tasks')}
              title={t('nav.tasks')}
              className={`flex h-10 w-10 items-center justify-center rounded-xl transition-colors ${
                currentTab === 'tasks'
                  ? 'bg-[var(--accent)] text-white'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
              }`}
            >
              <Sliders className="h-5 w-5" />
            </button>
            <button
              onClick={() => setCurrentTab('alarms')}
              title={t('nav.alarms')}
              className={`flex h-10 w-10 items-center justify-center rounded-xl transition-colors ${
                currentTab === 'alarms'
                  ? 'bg-[var(--accent)] text-white'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
              }`}
            >
              <AlertCircle className="h-5 w-5" />
            </button>
            <button
              onClick={() => setCurrentTab('oplog')}
              title={t('nav.oplog')}
              className={`flex h-10 w-10 items-center justify-center rounded-xl transition-colors ${
                currentTab === 'oplog'
                  ? 'bg-[var(--accent)] text-white'
                  : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)]'
              }`}
            >
              <FileText className="h-5 w-5" />
            </button>
          </nav>
        </div>

        {/* 底部控制区：多语言切换 + 主题切换 */}
        <div className="flex flex-col items-center gap-3">
          <button
            onClick={cycleLocale}
            title={`${t('actions.edit')} ${locale}`}
            className="flex h-10 w-10 flex-col items-center justify-center rounded-xl text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)]"
          >
            <Globe className="h-4 w-4" />
            <span className="text-[9px] font-bold uppercase">
              {locale === 'zh-CN' ? '简' : locale === 'zh-TW' ? '繁' : 'EN'}
            </span>
          </button>

          <button
            onClick={toggleTheme}
            title={isDark ? t('theme.toLight') : t('theme.toDark')}
            className="flex h-10 w-10 items-center justify-center rounded-xl text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)]"
          >
            {isDark ? (
              <Sun className="h-5 w-5 text-amber-400" />
            ) : (
              <Moon className="h-5 w-5 text-[var(--text-secondary)]" />
            )}
          </button>
        </div>
      </aside>

      {/* 主工作视口 */}
      <main className="flex flex-1 flex-col overflow-hidden p-4">
        {currentTab === 'live' && <LivePage />}
        {currentTab === 'tasks' && <TasksPage />}
        {currentTab === 'alarms' && <AlarmsPage />}
        {currentTab === 'oplog' && <OplogPage />}
      </main>
    </div>
  )
}
