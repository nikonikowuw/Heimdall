import React, { useEffect, useState } from 'react'
import { ArrowRight, Eye, EyeOff, KeyRound, Lock, Moon, Sun, User, Wand2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LocaleDropdown } from '../../components/LocaleDropdown'
import { useTheme } from '../../hooks/use-theme'
import { authApi } from '../../lib/api'
import { getRememberedUser, useAuthStore } from '../../stores/auth'
import { CursorRing } from './components/CursorRing'
import { GargantuaCanvas } from './components/GargantuaCanvas'

interface ToastInfo {
  id: number
  type: 'success' | 'error' | 'info'
  title: string
  message: string
}

export const LoginPage: React.FC = () => {
  const { t } = useTranslation('auth')
  const login = useAuthStore((state) => state.login)

  // 初始化状态与模式
  const [isInitialized, setIsInitialized] = useState<boolean | null>(null)
  const [username, setUsername] = useState(() => getRememberedUser())
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [remember, setRemember] = useState(() => Boolean(getRememberedUser()))
  const [loading, setLoading] = useState(false)
  const { isDark, toggleTheme } = useTheme()
  const [toasts, setToasts] = useState<ToastInfo[]>([])

  const handleFpsUpdate = React.useCallback((newFps: number) => {
    const el = document.getElementById('webgl-fps-badge')
    if (el) {
      el.textContent = `${newFps} FPS`
    }
  }, [])

  const addToast = (type: 'success' | 'error' | 'info', title: string, message: string) => {
    const id = Date.now()
    setToasts((prev) => [...prev, { id, type, title, message }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, 3500)
  }

  // 探活后端管理员账号初始化状态
  useEffect(() => {
    let mounted = true
    authApi
      .getInitStatus()
      .then((status) => {
        if (mounted) {
          setIsInitialized(status.initialized)
          if (!status.initialized) {
            addToast('info', 'SETUP REQUIRED', t('setupSubtitle'))
          }
        }
      })
      .catch(() => {
        // 网络尚未连通或本地静态环境，默认进入登录模式
        if (mounted) {
          setIsInitialized(true)
        }
      })
    return () => {
      mounted = false
    }
  }, [t])

  // 键盘快捷键支持：按 T 键快速切换日/夜模式
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement) return
      if (e.key === 't' || e.key === 'T') {
        toggleTheme()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [toggleTheme])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    const trimmedUsername = username.trim()
    if (!trimmedUsername || !password) {
      addToast('error', 'AUTH ERROR', t('loginError'))
      return
    }

    // 开箱向导分支：密码确认与强度校验
    if (isInitialized === false) {
      if (password.length < 6) {
        addToast('error', 'PASSWORD TOO SHORT', t('passwordLengthError'))
        return
      }
      if (password !== confirmPassword) {
        addToast('error', 'MISMATCH', t('passwordMismatch'))
        return
      }

      setLoading(true)
      try {
        const res = await authApi.initialize({
          username: trimmedUsername,
          password,
        })
        addToast('success', 'SYSTEM INITIALIZED', t('setupSuccess'))
        setTimeout(() => {
          login(res.accessToken, res.username, remember)
        }, 600)
      } catch (err: unknown) {
        setLoading(false)
        const msg = err instanceof Error ? err.message : t('loginError')
        addToast('error', 'INIT FAILED', msg)
      }
      return
    }

    // 正常登录分支
    setLoading(true)
    try {
      const res = await authApi.login({
        username: trimmedUsername,
        password,
      })
      addToast('success', 'SESSION GRANTED', t('loginSuccess'))
      setTimeout(() => {
        login(res.accessToken, res.username, remember)
      }, 600)
    } catch (err: unknown) {
      setLoading(false)
      const msg = err instanceof Error ? err.message : t('loginError')
      addToast('error', 'ACCESS DENIED', msg)
    }
  }

  return (
    <div className="relative flex min-h-screen w-full flex-col justify-between overflow-x-hidden font-sans antialiased select-none">
      {/* 视觉底层：相对论黑洞/白洞 WebGL 物理渲染器 */}
      <GargantuaCanvas isDark={isDark} onFpsUpdate={handleFpsUpdate} />

      {/* 氛围渐变柔和暗角 */}
      <div
        aria-hidden="true"
        className="pointer-events-none fixed inset-0 z-[1] bg-[radial-gradient(ellipse_75%_65%_at_50%_48%,transparent_35%,rgba(15,23,42,0.06)_100%)] dark:bg-[radial-gradient(ellipse_75%_65%_at_50%_48%,transparent_30%,rgba(0,0,0,0.55)_100%)]"
      />

      {/* 顶层标定微网格 */}
      <div className="calibration-grid pointer-events-none fixed inset-0 z-10 opacity-30" />

      {/* 伴生物理柔光环 */}
      <CursorRing />

      {/* 全局 HUD Toast 浮层 */}
      <div className="pointer-events-none fixed top-5 right-5 z-50 flex w-[calc(100vw-2.5rem)] max-w-sm flex-col gap-2.5">
        {toasts.map((toast) => (
          <div
            key={toast.id}
            className={`lens-glass pointer-events-auto flex items-start gap-3 rounded-2xl border p-4 shadow-xl backdrop-blur-xl ${
              toast.type === 'success'
                ? 'border-emerald-500/30 text-emerald-600 dark:text-emerald-400'
                : toast.type === 'error'
                  ? 'border-rose-500/30 text-rose-600 dark:text-rose-400'
                  : 'border-indigo-500/30 text-indigo-600 dark:text-cyan-400'
            }`}
          >
            <div className="flex-1">
              <div className="font-mono text-[10px] font-bold tracking-wider uppercase">
                {toast.title}
              </div>
              <div className="mt-0.5 text-xs text-[var(--text-primary)]">{toast.message}</div>
            </div>
          </div>
        ))}
      </div>

      {/* 全局通栏 Header：左上角 Logo，右上角全局视口控制器（语言/FPS/主题滑块） */}
      <header className="pointer-events-none fixed inset-x-0 top-0 z-30 flex items-center justify-between px-6 py-5 sm:px-10 sm:py-6 lg:px-12">
        {/* 左上角：官方品牌标识 */}
        <div className="pointer-events-auto flex items-center">
          <img
            src={isDark ? '/logo-horizontal-dark.svg' : '/logo-horizontal-light.svg'}
            alt="Heimdall"
            className="h-8.5 w-auto transition-transform select-none hover:scale-[1.02] sm:h-9"
          />
        </div>

        {/* 右上角：全局视口控制组（FPS 标尺 + 语言自由选择下拉 + 日/夜胶囊滑块） */}
        <div className="pointer-events-auto flex items-center gap-3">
          {/* 实时 FPS 指示 */}
          <span
            id="webgl-fps-badge"
            className="rounded-full border border-black/5 bg-white/60 px-2.5 py-1 font-mono text-[10px] text-slate-500 tabular-nums shadow-sm backdrop-blur-md select-none dark:border-white/10 dark:bg-black/40 dark:text-slate-400"
          >
            60 FPS
          </span>

          {/* 自由语言选择下拉菜单 */}
          <LocaleDropdown placement="bottom-right" />

          {/* 右上角精致日/暗微滑块 */}
          <button
            type="button"
            onClick={toggleTheme}
            role="switch"
            aria-checked={isDark}
            aria-label="Toggle Theme: Light / Dark"
            title="切换亮色 / 暗色主题"
            className="group relative h-[28px] w-[54px] shrink-0 cursor-pointer rounded-full border border-slate-300 bg-slate-200/90 p-[2.5px] shadow-inner transition-colors duration-300 select-none focus:outline-none dark:border-slate-700/80 dark:bg-slate-900/90"
          >
            <div className="pointer-events-none absolute inset-0 flex items-center justify-between px-2 text-[10px] opacity-50">
              <Sun className="h-3 w-3 text-amber-500" />
              <Moon className="h-3 w-3 text-indigo-400" />
            </div>
            <div
              className={`relative z-10 flex h-[22px] w-[22px] items-center justify-center rounded-full shadow-md transition-all duration-300 ease-[cubic-bezier(0.34,1.56,0.64,1)] ${
                isDark
                  ? 'translate-x-[26px] bg-indigo-600 text-white'
                  : 'translate-x-0 bg-white text-slate-800'
              }`}
            >
              {isDark ? (
                <Moon className="h-3.5 w-3.5 text-white" />
              ) : (
                <Sun className="h-3.5 w-3.5 text-amber-500" />
              )}
            </div>
          </button>
        </div>
      </header>

      {/* 核心双栏架构：左侧边缘管线状态 + 右侧先锋悬浮控制吊舱 */}
      <div className="pointer-events-none relative z-20 flex min-h-screen w-full flex-col items-center justify-between px-6 pt-20 pb-6 sm:px-10 sm:pb-8 lg:flex-row lg:px-12 lg:pt-24">
        {/* 左侧：边缘媒体与 AI 管线底层状态 */}
        <div className="flex min-h-[38vh] w-full flex-col justify-between select-none lg:min-h-[calc(100vh-8rem)] lg:flex-1">
          <div />
          <div className="flex flex-1 items-center justify-center" />
          <div className="flex flex-wrap items-center gap-4 font-mono text-[10px] tracking-widest text-slate-400/80 uppercase select-none dark:text-slate-500/80">
            <span className="flex items-center gap-1.5">
              <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-500" />
              <span>{t('opticalSensor')}</span>
            </span>
            <span className="hidden text-slate-300 sm:inline dark:text-slate-800">|</span>
            <span className="hidden sm:inline">{t('kerrMetric')}</span>
            <span className="hidden text-slate-300 md:inline dark:text-slate-800">|</span>
            <span className="hidden md:inline">{t('directBus')}</span>
          </div>
        </div>

        {/* 右侧：先锋悬浮控制吊舱 */}
        <div className="pointer-events-auto my-auto w-full lg:mr-2 lg:w-[410px] xl:mr-6 xl:w-[430px]">
          <aside
            id="command-dock"
            className="lens-glass relative w-full overflow-hidden rounded-3xl p-6 shadow-2xl backdrop-blur-3xl transition-all duration-300 sm:p-7"
          >
            <div className="pointer-events-none absolute -top-20 left-1/2 h-32 w-64 -translate-x-1/2 rounded-full bg-gradient-to-b from-indigo-500/20 via-pink-500/10 to-transparent blur-2xl" />

            <div className="relative z-10 flex items-center justify-between border-b border-black/5 pb-4 dark:border-white/5">
              <div className="flex items-center gap-2">
                <span className="relative flex h-2 w-2">
                  <span
                    className={`absolute inline-flex h-full w-full animate-ping rounded-full ${
                      isInitialized === false ? 'bg-amber-400' : 'bg-emerald-400'
                    } opacity-75`}
                  />
                  <span
                    className={`relative inline-flex h-2 w-2 rounded-full ${
                      isInitialized === false ? 'bg-amber-500' : 'bg-emerald-500'
                    }`}
                  />
                </span>
                <span className="font-mono text-[10px] font-semibold tracking-widest text-slate-700 uppercase dark:text-slate-300">
                  {isInitialized === false ? 'OOBE // FIRST BOOT' : t('nodeStatus')}
                </span>
              </div>

              <span className="font-mono text-[10px] text-slate-400 dark:text-slate-500">
                REV. 2026.1
              </span>
            </div>

            {/* 控制台核心面板 */}
            <div className="my-auto py-5">
              <div className="mb-5 space-y-1">
                <h2 className="font-display flex items-center gap-2 text-2xl font-bold tracking-tight text-slate-900 dark:text-white">
                  {isInitialized === false && <Wand2 className="h-5 w-5 text-amber-500" />}
                  <span>{isInitialized === false ? t('setupTitle') : t('title')}</span>
                </h2>
                <p className="text-xs leading-relaxed text-slate-500 dark:text-slate-400">
                  {isInitialized === false ? t('setupSubtitle') : t('subtitle')}
                </p>
              </div>

              {/* 表单 */}
              <form onSubmit={handleSubmit} className="space-y-4">
                {/* 用户名 */}
                <div>
                  <label
                    htmlFor="username"
                    className="mb-1.5 block text-xs font-medium text-slate-700 dark:text-slate-300"
                  >
                    {t('operatorId')}
                  </label>
                  <div className="relative">
                    <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3.5 text-slate-400">
                      <User className="h-4 w-4" />
                    </div>
                    <input
                      type="text"
                      id="username"
                      name="username"
                      required
                      value={username}
                      onChange={(e) => setUsername(e.target.value)}
                      placeholder={isInitialized === false ? 'admin' : t('operatorId')}
                      className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-4 pl-10 text-sm text-[var(--text-primary)] placeholder:text-slate-400 focus:border-cyan-400 focus:ring-1 focus:ring-cyan-400/50 focus:outline-none dark:border-white/10 dark:bg-white/[0.04] dark:placeholder:text-slate-600"
                    />
                  </div>
                </div>

                {/* 密码 */}
                <div>
                  <label
                    htmlFor="password"
                    className="mb-1.5 block text-xs font-medium text-slate-700 dark:text-slate-300"
                  >
                    {isInitialized === false ? t('newPassword') : t('password')}
                  </label>
                  <div className="relative">
                    <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3.5 text-slate-400">
                      <KeyRound className="h-4 w-4" />
                    </div>
                    <input
                      type={showPassword ? 'text' : 'password'}
                      id="password"
                      name="password"
                      required
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      placeholder="••••••••••••"
                      className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-10 pl-10 text-sm text-[var(--text-primary)] placeholder:text-slate-400 focus:border-pink-400 focus:ring-1 focus:ring-pink-400/50 focus:outline-none dark:border-white/10 dark:bg-white/[0.04] dark:placeholder:text-slate-600"
                    />
                    <button
                      type="button"
                      onClick={() => setShowPassword(!showPassword)}
                      aria-label="Toggle password visibility"
                      className="absolute inset-y-0 right-0 flex cursor-pointer items-center pr-3.5 text-slate-400 transition-colors hover:text-slate-600 dark:hover:text-slate-200"
                    >
                      {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                </div>

                {/* 开箱向导模式：确认密码 */}
                {isInitialized === false && (
                  <div>
                    <label
                      htmlFor="confirmPassword"
                      className="mb-1.5 block text-xs font-medium text-slate-700 dark:text-slate-300"
                    >
                      {t('confirmPassword')}
                    </label>
                    <div className="relative">
                      <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3.5 text-slate-400">
                        <KeyRound className="h-4 w-4" />
                      </div>
                      <input
                        type={showPassword ? 'text' : 'password'}
                        id="confirmPassword"
                        name="confirmPassword"
                        required
                        value={confirmPassword}
                        onChange={(e) => setConfirmPassword(e.target.value)}
                        placeholder="••••••••••••"
                        className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-10 pl-10 text-sm text-[var(--text-primary)] placeholder:text-slate-400 focus:border-indigo-400 focus:ring-1 focus:ring-indigo-400/50 focus:outline-none dark:border-white/10 dark:bg-white/[0.04] dark:placeholder:text-slate-600"
                      />
                    </div>
                  </div>
                )}

                {/* 记住凭证选项（仅正常登录） */}
                {isInitialized !== false && (
                  <div className="flex items-center justify-between pt-0.5">
                    <label className="flex cursor-pointer items-center gap-2 select-none">
                      <input
                        type="checkbox"
                        checked={remember}
                        onChange={(e) => setRemember(e.target.checked)}
                        className="h-4 w-4 rounded border-slate-300 bg-slate-100 text-indigo-600 accent-indigo-500 dark:border-slate-700 dark:bg-slate-900"
                      />
                      <span className="text-xs text-slate-600 dark:text-slate-400">
                        {t('remember')}
                      </span>
                    </label>
                    <span className="font-mono text-[10px] text-slate-400 dark:text-slate-500">
                      {t('keystoreOk')}
                    </span>
                  </div>
                )}

                {/* 提交按钮 */}
                <div className="pt-2">
                  <button
                    type="submit"
                    disabled={loading}
                    className="font-display flex w-full cursor-pointer items-center justify-center gap-2 rounded-xl bg-gradient-to-r from-indigo-600 via-purple-600 to-cyan-500 px-4 py-3 text-xs font-bold tracking-wider text-white uppercase shadow-lg shadow-indigo-600/25 transition-all duration-200 hover:opacity-95 active:scale-[0.99] disabled:opacity-50"
                  >
                    <ArrowRight className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
                    <span>
                      {loading
                        ? t('submitting')
                        : isInitialized === false
                          ? t('setupSubmit')
                          : t('submit')}
                    </span>
                  </button>
                </div>
              </form>

              {/* 工业技术标尺三联 */}
              <div className="mt-6 grid grid-cols-3 gap-2 border-t border-black/5 pt-4 text-center select-none dark:border-white/5">
                <div className="rounded-lg border border-black/5 bg-black/[0.02] p-2 dark:border-white/5 dark:bg-white/[0.02]">
                  <div className="text-[10px] text-slate-400 dark:text-slate-500">
                    {t('ingestion')}
                  </div>
                  <div className="mt-0.5 text-xs font-semibold text-slate-700 dark:text-slate-300">
                    WebRTC/RTSP
                  </div>
                </div>
                <div className="rounded-lg border border-black/5 bg-black/[0.02] p-2 dark:border-white/5 dark:bg-white/[0.02]">
                  <div className="text-[10px] text-slate-400 dark:text-slate-500">
                    {t('pipeline')}
                  </div>
                  <div className="mt-0.5 text-xs font-semibold text-cyan-600 dark:text-cyan-400">
                    DMA-BUF
                  </div>
                </div>
                <div className="rounded-lg border border-black/5 bg-black/[0.02] p-2 dark:border-white/5 dark:bg-white/[0.02]">
                  <div className="text-[10px] text-slate-400 dark:text-slate-500">
                    {t('zeroCopy')}
                  </div>
                  <div className="mt-0.5 text-xs font-semibold text-indigo-600 dark:text-pink-400">
                    Zero-Copy
                  </div>
                </div>
              </div>
            </div>

            {/* 吊舱底部安全与版权 */}
            <div className="border-t border-black/5 pt-4 text-xs text-slate-500 dark:border-white/5">
              <div className="flex items-center justify-between font-mono text-[11px] text-slate-400 select-none dark:text-slate-500">
                <span className="flex items-center gap-1.5">
                  <Lock className="h-3.5 w-3.5 text-emerald-500" />
                  <span>{t('security')}</span>
                </span>
                <span>© 2026 Heimdall</span>
              </div>
            </div>
          </aside>
        </div>
      </div>
    </div>
  )
}
