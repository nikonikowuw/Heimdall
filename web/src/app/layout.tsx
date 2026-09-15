import { Suspense, lazy, useState } from 'react'
import {
  AlertCircle,
  Cpu,
  FileText,
  Keyboard,
  Loader2,
  LogOut,
  Monitor,
  Moon,
  Settings,
  Sliders,
  Sun,
  Users,
  Video,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ChangePasswordModal } from '../components/ChangePasswordModal'
import { LocaleDropdown } from '../components/LocaleDropdown'
import { ShortcutsModal } from '../components/ShortcutsModal'
import { useGlobalShortcuts } from '../hooks/use-global-shortcuts'
import { useTheme } from '../hooks/use-theme'
import { authApi } from '../lib/api'
import { useAuthStore } from '../stores/auth'

const LoginPage = lazy(() =>
  import('../features/auth/LoginPage').then((m) => ({ default: m.LoginPage })),
)
const LivePage = lazy(() =>
  import('../features/live/LivePage').then((m) => ({ default: m.LivePage })),
)
const CamerasPage = lazy(() =>
  import('../features/cameras/CamerasPage').then((m) => ({ default: m.CamerasPage })),
)
const TasksPage = lazy(() =>
  import('../features/tasks/TasksPage').then((m) => ({ default: m.TasksPage })),
)
const AlgorithmsPage = lazy(() =>
  import('../features/algorithms/AlgorithmsPage').then((m) => ({ default: m.AlgorithmsPage })),
)
const PersonnelPage = lazy(() =>
  import('../features/personnel/PersonnelPage').then((m) => ({ default: m.PersonnelPage })),
)
const AlarmsPage = lazy(() =>
  import('../features/alarms/AlarmsPage').then((m) => ({ default: m.AlarmsPage })),
)
const OplogPage = lazy(() =>
  import('../features/oplog/OplogPage').then((m) => ({ default: m.OplogPage })),
)
const SettingsPage = lazy(() =>
  import('../features/system/SettingsPage').then((m) => ({ default: m.SettingsPage })),
)

export type NavTab =
  'live' | 'cameras' | 'tasks' | 'algorithms' | 'personnel' | 'alarms' | 'oplog' | 'system'

const NAV_ITEMS: { tab: NavTab; icon: typeof Monitor; labelKey: string }[] = [
  { tab: 'live', icon: Monitor, labelKey: 'nav.live' },
  { tab: 'cameras', icon: Video, labelKey: 'nav.cameras' },
  { tab: 'tasks', icon: Sliders, labelKey: 'nav.tasks' },
  { tab: 'algorithms', icon: Cpu, labelKey: 'nav.algorithms' },
  { tab: 'personnel', icon: Users, labelKey: 'nav.personnel' },
  { tab: 'alarms', icon: AlertCircle, labelKey: 'nav.alarms' },
  { tab: 'oplog', icon: FileText, labelKey: 'nav.oplog' },
]

export function Layout() {
  const { t } = useTranslation(['common', 'auth'])
  const { isAuthenticated, logout, username } = useAuthStore()
  const [currentTab, setCurrentTab] = useState<NavTab>('live')
  const [targetTaskCameraId, setTargetTaskCameraId] = useState<string | null>(null)
  const [isPasswordModalOpen, setIsPasswordModalOpen] = useState(false)
  const [isShortcutsOpen, setIsShortcutsOpen] = useState(false)
  const { isDark, toggleTheme } = useTheme()
  const loadingFallback = (
    <div
      className="flex h-screen w-screen items-center justify-center bg-[var(--bg-primary)]"
      role="status"
      aria-label={t('loading')}
    >
      <Loader2 className="h-6 w-6 animate-spin text-[var(--accent)]" aria-hidden="true" />
    </div>
  )

  useGlobalShortcuts({
    currentTab,
    onSelectTab: (tab) => setCurrentTab(tab),
    onToggleTheme: toggleTheme,
    onOpenShortcutsHelp: () => setIsShortcutsOpen(true),
    enabled: isAuthenticated,
  })

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
    return (
      <Suspense fallback={loadingFallback}>
        <LoginPage />
      </Suspense>
    )
  }

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-[var(--bg-primary)] font-sans antialiased">
      {/* 左侧紧凑工具导航栏 */}
      <aside className="frosted-glass relative z-40 flex w-16 flex-col items-center justify-between py-4">
        <div className="flex flex-col items-center gap-5">
          {/* Logo 标志 */}
          <button
            type="button"
            onClick={() => setCurrentTab('live')}
            title="Heimdall"
            className="flex h-10 w-10 items-center justify-center rounded-xl transition-transform hover:scale-105 focus:outline-none"
          >
            <img
              src={isDark ? '/favicon-dark.svg' : '/favicon-light.svg'}
              alt="Heimdall"
              className="h-10 w-10 rounded-xl shadow-md"
            />
          </button>

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

          <button
            onClick={() => setIsShortcutsOpen(true)}
            title={`${t('shortcuts.title')} (?)`}
            className="nav-btn"
          >
            <Keyboard className="h-5 w-5" />
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
        <Suspense fallback={loadingFallback}>
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
          {currentTab === 'personnel' && <PersonnelPage />}
          {currentTab === 'alarms' && <AlarmsPage />}
          {currentTab === 'oplog' && <OplogPage />}
          {currentTab === 'system' && (
            <SettingsPage onOpenPasswordModal={() => setIsPasswordModalOpen(true)} />
          )}
        </Suspense>
      </main>
      {/* 修改密码模态框 */}
      <ChangePasswordModal
        isOpen={isPasswordModalOpen}
        onClose={() => setIsPasswordModalOpen(false)}
      />
      {/* 快捷键速查面板 */}
      <ShortcutsModal isOpen={isShortcutsOpen} onClose={() => setIsShortcutsOpen(false)} />
    </div>
  )
}
