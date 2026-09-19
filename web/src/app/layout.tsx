import { Suspense, useEffect, useRef, useState } from 'react'
import {
  AlertCircle,
  Cpu,
  FileText,
  KeyRound,
  Keyboard,
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
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { ChangePasswordModal } from '@/components/ChangePasswordModal'
import { LocaleDropdown } from '@/components/LocaleDropdown'
import { ShortcutsModal } from '@/components/ShortcutsModal'
import { Toaster } from '@/components/ui/Toast'
import { RouteFallback } from '@/components/ui/RouteFallback'
import { LoginPage } from '@/features/auth/LoginPage'
import {
  AlarmsPage,
  AlgorithmsPage,
  CamerasPage,
  LivePage,
  OplogPage,
  PersonnelPage,
  SettingsPage,
  TasksPage,
  preloadDefaultTab,
} from './lazy-pages'
import { useGlobalShortcuts } from '@/hooks/use-global-shortcuts'
import { useTheme } from '@/hooks/use-theme'
import { authApi } from '@/lib/api'
import { useAuthStore } from '@/stores/auth'
import { type NavTab } from '@/types'

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
  const [userMenuOpen, setUserMenuOpen] = useState(false)
  const userMenuRef = useRef<HTMLDivElement | null>(null)
  const { isDark, toggleTheme } = useTheme()
  const reducedMotion = useReducedMotion()

  useDismissStack(userMenuOpen, () => setUserMenuOpen(false), { priority: 5 })

  useEffect(() => {
    if (!userMenuOpen) return
    const handleClickOutside = (e: MouseEvent) => {
      if (userMenuRef.current && !userMenuRef.current.contains(e.target as Node)) {
        setUserMenuOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => {
      document.removeEventListener('mousedown', handleClickOutside)
    }
  }, [userMenuOpen])

  // 鉴权通过后预取默认落地页，让工作区入场动画期间完成 chunk 下载，
  // 使用户感知不到懒加载带来的额外往返。
  useEffect(() => {
    if (isAuthenticated) preloadDefaultTab()
  }, [isAuthenticated])

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
        return <CamerasPage />
      case 'tasks':
        return (
          <TasksPage
            initialConfigCameraId={targetTaskCameraId}
            onNavigateToCameras={() => setCurrentTab('cameras')}
            onNavigateToAlgorithms={() => setCurrentTab('algorithms')}
          />
        )
      case 'algorithms':
        return (
          <AlgorithmsPage
            onNavigateToTask={(cameraId) => {
              setTargetTaskCameraId(cameraId)
              setCurrentTab('tasks')
            }}
          />
        )
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
      <Toaster />
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
            <LoginPage />
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

                {/* 用户身份与快捷控制胶囊 */}
                <div className="relative shrink-0" ref={userMenuRef}>
                  <button
                    type="button"
                    onClick={() => setUserMenuOpen((open) => !open)}
                    aria-label={t('auth:logoutTooltip', { username: username || 'admin' })}
                    title={username || 'admin'}
                    className="relative flex h-9 w-9 items-center justify-center rounded-xl border border-[var(--border)] bg-gradient-to-b from-[var(--accent-soft)] to-[var(--bg-secondary)] font-mono text-xs font-bold text-[var(--text-primary)] shadow-2xs transition-all hover:border-[var(--accent)] hover:shadow-xs focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
                  >
                    <span>{username ? username.charAt(0).toUpperCase() : 'A'}</span>
                    <span
                      aria-hidden="true"
                      className="absolute -right-0.5 -bottom-0.5 h-2.5 w-2.5 rounded-full border-2 border-[var(--bg-surface-solid)] bg-emerald-500 shadow-xs"
                    />
                  </button>

                  {userMenuOpen && (
                    <div
                      role="menu"
                      className="lens-glass animate-in fade-in slide-in-from-left-2 absolute bottom-0 left-full z-50 ml-3 min-w-[210px] overflow-hidden rounded-2xl border border-[var(--border)] p-2 shadow-2xl duration-150"
                    >
                      {/* 用户档案信息标牌 */}
                      <div className="flex items-center gap-3 p-2">
                        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--accent-soft)] font-mono text-xs font-bold text-[var(--accent)]">
                          {username ? username.charAt(0).toUpperCase() : 'A'}
                        </div>
                        <div className="min-w-0 flex-1">
                          <p className="truncate font-mono text-xs font-bold text-[var(--text-primary)]">
                            {username || 'admin'}
                          </p>
                          <div className="flex items-center gap-1.5 text-[10px] text-[var(--text-muted)]">
                            <span className="h-1.5 w-1.5 rounded-full bg-emerald-400" />
                            <span>
                              {t('auth:roleAdministrator', { defaultValue: 'Administrator' })}
                            </span>
                          </div>
                        </div>
                      </div>

                      <div className="my-1 border-t border-[var(--border)]" />

                      {/* 快捷操作 */}
                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setUserMenuOpen(false)
                          setIsPasswordModalOpen(true)
                        }}
                        className="flex w-full items-center gap-2 rounded-xl px-2.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      >
                        <KeyRound className="h-3.5 w-3.5 opacity-70" />
                        <span>{t('auth:changePassword', { defaultValue: '修改密码' })}</span>
                      </button>

                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setUserMenuOpen(false)
                          setCurrentTab('system')
                        }}
                        className="flex w-full items-center gap-2 rounded-xl px-2.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--accent)]"
                      >
                        <Settings className="h-3.5 w-3.5 opacity-70" />
                        <span>{t('nav.system', { defaultValue: '系统设置' })}</span>
                      </button>

                      <div className="my-1 border-t border-[var(--border)]" />

                      <button
                        type="button"
                        role="menuitem"
                        onClick={() => {
                          setUserMenuOpen(false)
                          handleLogout()
                        }}
                        className="flex w-full items-center gap-2 rounded-xl px-2.5 py-1.5 text-xs font-medium text-rose-500 transition-colors hover:bg-rose-500/10"
                      >
                        <LogOut className="h-3.5 w-3.5 opacity-80" />
                        <span>
                          {t('auth:logoutTooltip', {
                            username: username || 'admin',
                            defaultValue: '退出登录',
                          })}
                        </span>
                      </button>
                    </div>
                  )}
                </div>
              </div>
            </motion.aside>

            {/* 主工作视口 */}
            <motion.main
              initial={
                reducedMotion ? false : { y: 12, opacity: 0, scale: 0.992, filter: 'blur(6px)' }
              }
              animate={{ y: 0, opacity: 1, scale: 1, filter: 'blur(0px)' }}
              transition={{ duration: 0.42, ease: [0.16, 1, 0.3, 1], delay: 0.08 }}
              className="content-ambient hud-viewport-frame relative z-10 flex flex-1 flex-col overflow-hidden p-4"
            >
              <div className="flex h-full w-full flex-1 flex-col overflow-hidden">
                <Suspense fallback={<RouteFallback />}>{renderTabContent()}</Suspense>
              </div>
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
