import React, { useState } from 'react'
import {
  AlertCircle,
  Camera,
  FileText,
  KeyRound,
  LogOut,
  Moon,
  Sliders,
  Sun,
  Video,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ChangePasswordModal } from '../components/ChangePasswordModal'
import { LocaleDropdown } from '../components/LocaleDropdown'
import { AlarmsPage } from '../features/alarms/AlarmsPage'
import { LoginPage } from '../features/auth'
import { LivePage } from '../features/live/LivePage'
import { OplogPage } from '../features/oplog/OplogPage'
import { TasksPage } from '../features/tasks/TasksPage'
import { useTheme } from '../hooks/use-theme'
import { authApi } from '../lib/api'
import { useAuthStore } from '../stores/auth'

export type NavTab = 'live' | 'tasks' | 'alarms' | 'oplog'

export const Layout: React.FC = () => {
  const { t } = useTranslation(['common', 'auth'])
  const { isAuthenticated, logout, username } = useAuthStore()
  const [currentTab, setCurrentTab] = useState<NavTab>('live')
  const [isPasswordModalOpen, setIsPasswordModalOpen] = useState(false)
  const { isDark, toggleTheme } = useTheme()

  const handleLogout = async () => {
    try {
      await authApi.logout()
    } catch {
      // 忽略登出请求网络异常
    } finally {
      logout()
    }
  }

  // 未登录状态展示先锋高奢登录页
  if (!isAuthenticated) {
    return <LoginPage />
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

        {/* 底部控制区：多语言自由下拉选择 + 主题切换 + 退出登录 */}
        <div className="flex flex-col items-center gap-3">
          <LocaleDropdown variant="icon" placement="right-bottom" />

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

          <button
            onClick={() => setIsPasswordModalOpen(true)}
            title={t('auth:changePasswordTooltip', { username: username || 'admin' })}
            className="flex h-10 w-10 items-center justify-center rounded-xl text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)]"
          >
            <KeyRound className="h-5 w-5" />
          </button>

          <button
            onClick={handleLogout}
            title={t('auth:logoutTooltip', { username: username || 'admin' })}
            className="flex h-10 w-10 items-center justify-center rounded-xl text-[var(--destructive)] transition-colors hover:bg-rose-500/10"
          >
            <LogOut className="h-5 w-5" />
          </button>
        </div>
      </aside>

      {/* 主工作视口 */}
      <main className="flex flex-1 flex-col overflow-hidden p-4">
        {currentTab === 'live' && <LivePage onNavigateToAlarms={() => setCurrentTab('alarms')} />}
        {currentTab === 'tasks' && <TasksPage />}
        {currentTab === 'alarms' && <AlarmsPage />}
        {currentTab === 'oplog' && <OplogPage />}
      </main>

      {/* 修改密码模态框 */}
      <ChangePasswordModal
        isOpen={isPasswordModalOpen}
        onClose={() => setIsPasswordModalOpen(false)}
      />
    </div>
  )
}
