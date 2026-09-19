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
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
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

interface NavActiveIndicatorProps {
  reducedMotion: boolean | null
}

function NavActiveIndicator({ reducedMotion }: NavActiveIndicatorProps): React.ReactElement {
  if (reducedMotion) {
    return <div className="absolute inset-0 rounded-xl bg-[var(--accent)]" />
  }
  return (
    <>
      <motion.div
        layoutId="nav-active-pill"
        className="absolute inset-0 rounded-xl bg-[var(--accent)] shadow-[0_0_16px_rgba(var(--accent-rgb),0.35)]"
        transition={{
          type: 'spring',
          stiffness: 480,
          damping: 36,
        }}
      />
      <motion.div
        layoutId="nav-active-rail"
        className="absolute top-2.5 bottom-2.5 -left-2 w-[3px] rounded-full bg-[var(--accent)] shadow-[0_0_8px_rgba(var(--accent-rgb),0.8)]"
        transition={{
          type: 'spring',
          stiffness: 480,
          damping: 36,
        }}
      />
    </>
  )
}

const NAV_ITEMS: { tab: NavTab; icon: typeof Monitor; labelKey: string }[] = [
  { tab: 'live', icon: Monitor, labelKey: 'nav.live' },
  { tab: 'cameras', icon: Video, labelKey: 'nav.cameras' },
  { tab: 'tasks', icon: Sliders, labelKey: 'nav.tasks' },
  { tab: 'algorithms', icon: Cpu, labelKey: 'nav.algorithms' },
  { tab: 'personnel', icon: Users, labelKey: 'nav.personnel' },
  { tab: 'alarms', icon: AlertCircle, labelKey: 'nav.alarms' },
  { tab: 'oplog', icon: FileText, labelKey: 'nav.oplog' },
]

export function Layout(): React.ReactElement {
  const { t } = useTranslation(['common', 'auth'])
  const { isAuthenticated, logout, username } = useAuthStore()
  const [currentTab, setCurrentTab] = useState<NavTab>('live')
  const [targetTaskCameraId, setTargetTaskCameraId] = useState<string | null>(null)
  const [isPasswordModalOpen, setIsPasswordModalOpen] = useState(false)
  const [isShortcutsOpen, setIsShortcutsOpen] = useState(false)
  const { isDark, toggleTheme } = useTheme()
  const reducedMotion = useReducedMotion()
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

  const renderTabContent = () => {
    switch (currentTab) {
      case 'live':
        return <LivePage onNavigateToAlarms={() => setCurrentTab('alarms')} />
      case 'cameras':
        return (
          <CamerasPage
            onNavigateToTasks={(camera) => {
              setTargetTaskCameraId(camera.cameraId)
              setCurrentTab('tasks')
            }}
          />
        )
      case 'tasks':
        return (
          <TasksPage
            initialConfigCameraId={targetTaskCameraId}
            onNavigateToCameras={() => setCurrentTab('cameras')}
            onNavigateToAlgorithms={() => setCurrentTab('algorithms')}
          />
        )
      case 'algorithms':
        return <AlgorithmsPage />
      case 'personnel':
        return <PersonnelPage />
      case 'alarms':
        return <AlarmsPage />
      case 'oplog':
        return <OplogPage />
      case 'system':
        return <SettingsPage onOpenPasswordModal={() => setIsPasswordModalOpen(true)} />
    }
  }

  return (
    <>
      <AnimatePresence mode="wait">
        {!isAuthenticated ? (
          <motion.div
            key="auth-gateway"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={
              reducedMotion
                ? { opacity: 0 }
                : {
                    opacity: 0,
                    scale: 0.98,
                    filter: 'blur(8px)',
                  }
            }
            transition={{
              duration: 0.26,
              ease: [0.16, 1, 0.3, 1],
            }}
            className="fixed inset-0 z-50 overflow-hidden bg-[var(--bg-primary)]"
          >
            <Suspense fallback={loadingFallback}>
              <LoginPage />
            </Suspense>
          </motion.div>
        ) : (
          <motion.div
            key="system-workspace"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{
              duration: 0.25,
              ease: [0.16, 1, 0.3, 1],
            }}
            className="saas-backdrop relative flex h-screen w-screen overflow-hidden font-sans antialiased"
          >
            {/* 全局微米物理噪点薄膜 (消除渐变色阶断层，赋予实体质感) */}
            <div className="saas-noise-overlay pointer-events-none fixed inset-0 z-0" />

            {/* 左侧紧凑工具导航栏 (带物理滑入) */}
            <motion.aside
              initial={reducedMotion ? false : { x: -20, opacity: 0 }}
              animate={{ x: 0, opacity: 1 }}
              transition={{ duration: 0.35, ease: [0.16, 1, 0.3, 1], delay: 0.04 }}
              className="frosted-glass relative z-40 flex w-16 shrink-0 flex-col items-center justify-between border-r border-[var(--border)] py-4 backdrop-blur-xl"
            >
              <div className="flex flex-col items-center gap-5">
                {/* Logo 标志 */}
                <button
                  type="button"
                  onClick={() => setCurrentTab('live')}
                  title="Heimdall"
                  className="flex h-10 w-10 items-center justify-center rounded-xl transition-transform hover:scale-105 focus:outline-none active:scale-95"
                >
                  <img
                    src={isDark ? '/favicon-dark.svg' : '/favicon-light.svg'}
                    alt="Heimdall"
                    className="h-10 w-10 rounded-xl shadow-md"
                  />
                </button>

                {/* 导航菜单 */}
                <nav className="flex flex-col gap-1.5">
                  {NAV_ITEMS.map(({ tab, icon: Icon, labelKey }) => {
                    const isActive = currentTab === tab
                    return (
                      <button
                        key={tab}
                        onClick={() => setCurrentTab(tab)}
                        title={t(labelKey, { defaultValue: tab })}
                        className={`nav-btn relative ${
                          isActive
                            ? 'text-white'
                            : 'text-[var(--text-secondary)] hover:text-[var(--accent)]'
                        }`}
                      >
                        {isActive && <NavActiveIndicator reducedMotion={reducedMotion} />}
                        <Icon className="relative z-10 h-5 w-5" />
                      </button>
                    )
                  })}
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
                  {isDark ? (
                    <Sun className="h-5 w-5 text-amber-400" />
                  ) : (
                    <Moon className="h-5 w-5" />
                  )}
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
                  className={`nav-btn relative ${
                    currentTab === 'system'
                      ? 'text-white'
                      : 'text-[var(--text-secondary)] hover:text-[var(--accent)]'
                  }`}
                >
                  {currentTab === 'system' && <NavActiveIndicator reducedMotion={reducedMotion} />}
                  <Settings className="relative z-10 h-5 w-5" />
                </button>

                <button
                  onClick={handleLogout}
                  title={t('auth:logoutTooltip', { username: username || 'admin' })}
                  className="nav-btn text-[var(--destructive)] hover:bg-rose-500/10 hover:text-[var(--destructive)]"
                >
                  <LogOut className="h-5 w-5" />
                </button>
              </div>
            </motion.aside>

            {/* 主工作视口 (带镜头级物理对焦浮现) */}
            <motion.main
              initial={
                reducedMotion ? false : { y: 12, opacity: 0, scale: 0.992, filter: 'blur(6px)' }
              }
              animate={{ y: 0, opacity: 1, scale: 1, filter: 'blur(0px)' }}
              transition={{ duration: 0.42, ease: [0.16, 1, 0.3, 1], delay: 0.08 }}
              className="content-ambient hud-viewport-frame relative z-10 flex flex-1 flex-col overflow-hidden p-4"
            >
              <Suspense fallback={loadingFallback}>
                <AnimatePresence mode="wait" initial={false}>
                  <motion.div
                    key={currentTab}
                    initial={reducedMotion ? { opacity: 1 } : { opacity: 0, y: 3 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={reducedMotion ? { opacity: 1 } : { opacity: 0, y: -3 }}
                    transition={{
                      duration: 0.15,
                      ease: [0.22, 1, 0.36, 1],
                    }}
                    className="flex h-full w-full flex-1 flex-col overflow-hidden"
                  >
                    {renderTabContent()}
                  </motion.div>
                </AnimatePresence>
              </Suspense>
            </motion.main>
            {/* 修改密码模态框 */}
            <ChangePasswordModal
              isOpen={isPasswordModalOpen}
              onClose={() => setIsPasswordModalOpen(false)}
            />
            {/* 快捷键速查面板 */}
            <ShortcutsModal isOpen={isShortcutsOpen} onClose={() => setIsShortcutsOpen(false)} />
          </motion.div>
        )}
      </AnimatePresence>
    </>
  )
}
