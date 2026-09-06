import { useState } from 'react'
import {
  AlertCircle,
  Cpu,
  FileText,
  Layers,
  LogOut,
  Monitor,
  Moon,
  Settings,
  Sliders,
  Sun,
  Video,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ChangePasswordModal } from '../components/ChangePasswordModal'
import { LocaleDropdown } from '../components/LocaleDropdown'
import { AlarmsPage } from '../features/alarms/AlarmsPage'
import { AlgorithmsPage } from '../features/algorithms'
import { LoginPage } from '../features/auth'
import { CamerasPage } from '../features/cameras'
import { LivePage } from '../features/live/LivePage'
import { OplogPage } from '../features/oplog/OplogPage'
import { SettingsPage } from '../features/system'
import { TasksPage } from '../features/tasks/TasksPage'
import { useTheme } from '../hooks/use-theme'
import { authApi } from '../lib/api'
import { useAuthStore } from '../stores/auth'

export type NavTab = 'live' | 'cameras' | 'tasks' | 'algorithms' | 'alarms' | 'oplog' | 'system'

const NAV_ITEMS: { tab: NavTab; icon: typeof Monitor; labelKey: string }[] = [
  { tab: 'live', icon: Monitor, labelKey: 'nav.live' },
  { tab: 'cameras', icon: Video, labelKey: 'nav.cameras' },
  { tab: 'tasks', icon: Sliders, labelKey: 'nav.tasks' },
  { tab: 'algorithms', icon: Cpu, labelKey: 'nav.algorithms' },
  { tab: 'alarms', icon: AlertCircle, labelKey: 'nav.alarms' },
  { tab: 'oplog', icon: FileText, labelKey: 'nav.oplog' },
]

export function Layout() {
  const { t } = useTranslation(['common', 'auth'])
  const { isAuthenticated, logout, username } = useAuthStore()
  const [currentTab, setCurrentTab] = useState<NavTab>('live')
  const [targetTaskCameraId, setTargetTaskCameraId] = useState<string | null>(null)
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
      <aside className="frosted-glass relative z-30 flex w-16 flex-col items-center justify-between py-4">
        <div className="flex flex-col items-center gap-5">
          {/* Logo 标志 */}
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[var(--accent)] text-white shadow-[var(--accent)]/25 shadow-lg">
            <Layers className="h-5 w-5" />
          </div>

          {/* 导航菜单 */}
          <nav className="flex flex-col gap-1.5">
            {NAV_ITEMS.map(({ tab, icon: Icon, labelKey }) => (
              <button
                key={tab}
                onClick={() => setCurrentTab(tab)}
                title={t(labelKey, { defaultValue: tab })}
                className={`nav-btn ${currentTab === tab ? 'active' : ''}`}
              >
                <Icon className="h-5 w-5" />
              </button>
            ))}
          </nav>
        </div>

        {/* 底部控制区 */}
        <div className="flex flex-col items-center gap-2">
          <LocaleDropdown variant="icon" placement="right-bottom" />

          <button
            onClick={toggleTheme}
            title={isDark ? t('theme.toLight') : t('theme.toDark')}
            className="nav-btn"
          >
            {isDark ? <Sun className="h-5 w-5 text-amber-400" /> : <Moon className="h-5 w-5" />}
          </button>

          <div className="my-1 h-px w-5 bg-[var(--border)]" />

          <button
            onClick={() => setCurrentTab('system')}
            title={t('nav.system', { defaultValue: '系统设置' })}
            className={`nav-btn ${currentTab === 'system' ? 'active' : ''}`}
          >
            <Settings className="h-5 w-5" />
          </button>

          <button
            onClick={handleLogout}
            title={t('auth:logoutTooltip', { username: username || 'admin' })}
            className="nav-btn text-[var(--destructive)] hover:bg-rose-500/10 hover:text-[var(--destructive)]"
          >
            <LogOut className="h-5 w-5" />
          </button>
        </div>
      </aside>

      {/* 主工作视口 */}
      <main className="content-ambient flex flex-1 flex-col overflow-hidden p-4">
        {currentTab === 'live' && <LivePage onNavigateToAlarms={() => setCurrentTab('alarms')} />}
        {currentTab === 'cameras' && (
          <CamerasPage
            onNavigateToTasks={(camera) => {
              setTargetTaskCameraId(camera.cameraId)
              setCurrentTab('tasks')
            }}
          />
        )}
        {currentTab === 'tasks' && (
          <TasksPage
            initialConfigCameraId={targetTaskCameraId}
            onNavigateToCameras={() => setCurrentTab('cameras')}
            onNavigateToAlgorithms={() => setCurrentTab('algorithms')}
          />
        )}
        {currentTab === 'algorithms' && <AlgorithmsPage />}
        {currentTab === 'alarms' && <AlarmsPage />}
        {currentTab === 'oplog' && <OplogPage />}
        {currentTab === 'system' && (
          <SettingsPage onOpenPasswordModal={() => setIsPasswordModalOpen(true)} />
        )}
      </main>
      {/* 修改密码模态框 */}
      <ChangePasswordModal
        isOpen={isPasswordModalOpen}
        onClose={() => setIsPasswordModalOpen(false)}
      />
    </div>
  )
}
