import { useCallback, useEffect, useRef, useState } from 'react'
import {
  ArrowRight,
  Check,
  Eye,
  EyeOff,
  KeyRound,
  Loader2,
  Moon,
  RefreshCw,
  Server,
  Sun,
  User,
  Wand2,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LocaleDropdown } from '@/components/LocaleDropdown'
import { useTheme } from '@/hooks/use-theme'
import { authApi } from '@/lib/api'
import { cn } from '@/lib/utils'
import { getRememberedUser, useAuthStore } from '@/stores/auth'
import { toast } from '@/stores/toast'
import { CursorRing } from './components/CursorRing'
import { GargantuaCanvas } from './components/GargantuaCanvas'
import { Starfield } from './components/Starfield'

type InitializationStatus = 'checking' | 'ready' | 'setup-required' | 'unavailable'

export function LoginPage(): React.ReactElement {
  const { t } = useTranslation('auth')
  const login = useAuthStore((state) => state.login)

  const [initializationStatus, setInitializationStatus] = useState<InitializationStatus>('checking')
  const [statusRetryKey, setStatusRetryKey] = useState(0)
  const [username, setUsername] = useState(() => getRememberedUser())
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [remember, setRemember] = useState(() => Boolean(getRememberedUser()))
  const [loading, setLoading] = useState(false)
  const [isSuccess, setIsSuccess] = useState(false)
  const [formError, setFormError] = useState<string | null>(null)
  const { isDark, toggleTheme } = useTheme()
  const usernameInputRef = useRef<HTMLInputElement | null>(null)
  const passwordInputRef = useRef<HTMLInputElement | null>(null)
  const confirmPasswordInputRef = useRef<HTMLInputElement | null>(null)
  const formErrorRef = useRef<HTMLDivElement | null>(null)
  const setupRequired = initializationStatus === 'setup-required'
  const canAuthenticate = initializationStatus === 'ready' || setupRequired

  const reportFieldError = (
    message: string,
    inputRef: React.RefObject<HTMLInputElement | null>,
  ) => {
    setFormError(message)
    inputRef.current?.focus()
  }

  const reportGeneralError = (message: string) => {
    setFormError(message)
    setTimeout(() => formErrorRef.current?.focus(), 0)
  }

  const handleAuthSuccess = (accessToken: string, authUsername: string) => {
    setIsSuccess(true)
    setTimeout(() => {
      login(accessToken, authUsername, remember)
    }, 160)
  }

  const handleFpsUpdate = useCallback((newFps: number) => {
    const el = document.getElementById('webgl-fps-badge')
    if (el) {
      el.textContent = newFps > 0 ? `${newFps} FPS` : '— FPS'
    }
  }, [])

  useEffect(() => {
    let mounted = true
    setInitializationStatus('checking')
    authApi
      .getInitStatus()
      .then((status) => {
        if (mounted) {
          setInitializationStatus(status.initialized ? 'ready' : 'setup-required')
        }
      })
      .catch(() => {
        if (mounted) setInitializationStatus('unavailable')
      })
    return () => {
      mounted = false
    }
  }, [statusRetryKey])

  // 键盘快捷键支持：按 T 键快速切换日/夜模式
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null
      if (target) {
        const tagName = target.tagName ? target.tagName.toUpperCase() : ''
        if (
          tagName === 'INPUT' ||
          tagName === 'TEXTAREA' ||
          tagName === 'SELECT' ||
          target.isContentEditable
        ) {
          return
        }
      }
      if (e.metaKey || e.ctrlKey || e.altKey) return
      if (e.key === 't' || e.key === 'T') {
        toggleTheme()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [toggleTheme])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!canAuthenticate) return

    const trimmedUsername = username.trim()
    if (!trimmedUsername) {
      reportFieldError(t('usernameRequired'), usernameInputRef)
      return
    }
    if (!password) {
      reportFieldError(t('passwordRequired'), passwordInputRef)
      return
    }

    if (setupRequired) {
      if (password.length < 6) {
        reportFieldError(t('passwordLengthError'), passwordInputRef)
        return
      }
      if (!confirmPassword) {
        reportFieldError(t('confirmPasswordRequired'), confirmPasswordInputRef)
        return
      }
      if (password !== confirmPassword) {
        reportFieldError(t('passwordMismatch'), confirmPasswordInputRef)
        return
      }

      setLoading(true)
      try {
        const res = await authApi.initialize({
          username: trimmedUsername,
          password,
        })
        toast.success(t('setupSuccess'), { title: t('toastSystemReady') })
        handleAuthSuccess(res.accessToken, res.username)
      } catch (err: unknown) {
        setLoading(false)
        reportGeneralError(err instanceof Error ? err.message : t('loginError'))
      }
      return
    }

    setLoading(true)
    try {
      const res = await authApi.login({
        username: trimmedUsername,
        password,
      })
      handleAuthSuccess(res.accessToken, res.username)
    } catch (err: unknown) {
      setLoading(false)
      reportGeneralError(err instanceof Error ? err.message : t('loginError'))
    }
  }

  let submitLabel = t('submit')
  if (loading) {
    submitLabel = t('submitting')
  } else if (setupRequired) {
    submitLabel = t('setupSubmit')
  }

  const statusView = (() => {
    switch (initializationStatus) {
      case 'checking':
        return {
          title: t('checkingTitle'),
          subtitle: t('checkingStatus'),
          badgeText: t('checkingStatus'),
          dotClass: 'auth-panel__status-dot--checking',
        }
      case 'unavailable':
        return {
          title: t('gatewayUnavailableTitle'),
          subtitle: t('gatewayUnavailableSubtitle'),
          badgeText: t('gatewayUnavailableTitle'),
          dotClass: 'auth-panel__status-dot--error',
        }
      case 'setup-required':
        return {
          title: t('setupTitle'),
          subtitle: t('setupSubtitle'),
          badgeText: t('firstBoot'),
          dotClass: 'auth-panel__status-dot--setup',
        }
      case 'ready':
      default:
        return {
          title: t('title'),
          subtitle: t('subtitle'),
          badgeText: t('nodeStatus'),
          dotClass: '',
        }
    }
  })()

  return (
    <main className="auth-shell auth-accent relative flex min-h-dvh w-full flex-col justify-between overflow-x-hidden font-sans antialiased">
      {/* 视觉底层：相对论黑洞/白洞 WebGL 物理渲染器 */}
      <GargantuaCanvas isDark={isDark} onFpsUpdate={handleFpsUpdate} />

      {/* 氛围渐变柔和暗角 */}
      <div aria-hidden="true" className="auth-vignette" />

      {/*
        动态极光层：承接页面动感，黑洞保持原样不动。
        首个渲染帧即由 CSS 动画驱动，主线程零参与；
        仅驱动 transform / opacity，全程位于 GPU 合成层。
      */}
      <div aria-hidden="true" className="auth-aurora">
        <div className="auth-aurora__blob auth-aurora__blob--primary" />
        <div className="auth-aurora__blob auth-aurora__blob--secondary" />
        <div className="auth-aurora__blob auth-aurora__blob--tertiary" />
      </div>

      {/* 远方星场：独立于 WebGL 黑洞的可见闪烁层，中央留出奇点与吸积盘空间 */}
      <Starfield />

      {/* 顶层标定微网格 */}
      <div className="auth-grid pointer-events-none fixed inset-0 z-10" />

      {/* 伴生物理柔光环 (仅在登录/初始化科技网关启用，支持 prefers-reduced-motion) */}
      <CursorRing />

      {/* 全局通栏 Header：左上角 Logo，右上角全局视口控制器（语言/FPS/主题滑块） */}
      <header className="auth-header pointer-events-none fixed inset-x-0 top-0 z-30 flex items-center justify-between">
        {/* 左上角：官方品牌标识 */}
        <div className="pointer-events-auto flex items-center">
          <img
            src={isDark ? '/logo-horizontal-dark.svg' : '/logo-horizontal-light.svg'}
            alt="Heimdall"
            className="auth-brand select-none"
          />
        </div>

        {/* 右上角：全局视口控制组（FPS 标尺 + 语言自由选择下拉 + 日/夜胶囊滑块） */}
        <div className="auth-header__controls pointer-events-auto">
          {/* 实时 FPS 指示 */}
          <span id="webgl-fps-badge" className="auth-fps-badge select-none">
            — FPS
          </span>

          {/* 自由语言选择下拉菜单 */}
          <LocaleDropdown placement="bottom-right" triggerClassName="auth-locale-trigger" />

          {/* 右上角精致日/暗微滑块 */}
          <button
            type="button"
            onClick={toggleTheme}
            role="switch"
            aria-checked={isDark}
            aria-label={t('themeToggle')}
            title={t('themeToggle')}
            className="auth-theme-toggle"
          >
            <div className="pointer-events-none flex items-center justify-between">
              <span className="auth-theme-toggle__icon auth-theme-toggle__icon--sun">
                <Sun className="h-3 w-3" />
              </span>
              <span className="auth-theme-toggle__icon auth-theme-toggle__icon--moon">
                <Moon className="h-3 w-3" />
              </span>
            </div>
            <div
              className={`auth-theme-toggle__thumb ${isDark ? 'auth-theme-toggle__thumb--dark' : 'auth-theme-toggle__thumb--light'}`}
            >
              {isDark ? <Moon className="h-3.5 w-3.5" /> : <Sun className="h-3.5 w-3.5" />}
            </div>
          </button>
        </div>
      </header>

      {/* 核心双栏架构：左侧边缘管线拓扑遥测 + 右侧先锋悬浮控制吊舱 */}
      <div className="pointer-events-none relative z-20 flex min-h-dvh w-full flex-col items-center justify-between px-6 pt-20 pb-6 sm:px-10 sm:pb-8 lg:flex-row lg:px-12 lg:pt-24">
        {/* 左侧：完全通透，把视觉舞台全部还给 Gargantua 物理黑洞 */}
        <div className="auth-stage hidden min-h-[38vh] w-full flex-col justify-between lg:flex lg:min-h-[calc(100dvh-8rem)] lg:flex-1">
          <div className="auth-stage-label hidden lg:flex">
            <span className="auth-stage-label__line" />
            <span>{t('terminal')}</span>
          </div>
          <div className="flex flex-1 items-center justify-center" />

          {/* 底栏运行参数状态 */}
          <div className="auth-stage-telemetry select-none">
            <span className="auth-stage-telemetry__status">
              <span className="auth-stage-telemetry__dot motion-safe:animate-pulse" />
              <span>{t('opticalSensor')}</span>
            </span>
            <span className="auth-stage-telemetry__divider hidden sm:inline">/</span>
            <span className="hidden sm:inline">{t('kerrMetric')}</span>
            <span className="auth-stage-telemetry__divider hidden md:inline">/</span>
            <span className="hidden md:inline">{t('directBus')}</span>
          </div>
        </div>

        {/* 右侧：先锋悬浮控制吊舱 */}
        <div className="pointer-events-auto my-auto w-full max-w-[460px] lg:mr-2 xl:mr-6">
          <section id="command-dock" aria-labelledby="auth-title" className="auth-panel">
            <div className="auth-panel__corner auth-panel__corner--top-left" />
            <div className="auth-panel__corner auth-panel__corner--top-right" />
            <div className="auth-panel__corner auth-panel__corner--bottom-left" />
            <div className="auth-panel__corner auth-panel__corner--bottom-right" />

            <div
              className={`auth-panel__accent-line ${isSuccess ? 'auth-panel__accent-line--success' : ''}`}
            />

            <div className="auth-panel__halo" />

            <div className="auth-panel__header">
              <div className="auth-panel__status">
                <span
                  className={cn(
                    'auth-panel__status-dot motion-safe:animate-pulse',
                    statusView.dotClass,
                  )}
                />
                <span>{statusView.badgeText}</span>
              </div>

              <span className="auth-panel__revision">REV. 2026.1</span>
            </div>

            <div className="auth-panel__body">
              <div className="auth-panel__intro">
                <div className="auth-panel__eyebrow">
                  <span className="auth-panel__eyebrow-mark" />
                  <span>{t('terminal')}</span>
                </div>
                <h1 id="auth-title" className="auth-panel__title">
                  {setupRequired && <Wand2 className="auth-setup-icon h-5 w-5" />}
                  <span>{statusView.title}</span>
                </h1>
                <p className="auth-panel__subtitle">{statusView.subtitle}</p>
              </div>

              <form
                onSubmit={handleSubmit}
                noValidate
                className="space-y-4"
                aria-busy={loading || initializationStatus === 'checking'}
              >
                {initializationStatus === 'checking' && (
                  <div className="auth-inline-state" role="status" aria-live="polite">
                    <Loader2 className="h-4 w-4 motion-safe:animate-spin" />
                    <span>{t('checkingStatus')}</span>
                  </div>
                )}
                {initializationStatus === 'unavailable' && (
                  <div className="auth-form-error" role="alert">
                    <p>{t('gatewayUnavailableMessage')}</p>
                    <button
                      type="button"
                      className="auth-retry"
                      onClick={() => setStatusRetryKey((previous) => previous + 1)}
                    >
                      <RefreshCw className="mr-2 h-4 w-4" />
                      {t('retryStatus')}
                    </button>
                  </div>
                )}
                {canAuthenticate && (
                  <>
                    {formError && (
                      <div
                        ref={formErrorRef}
                        className="auth-form-error"
                        role="alert"
                        aria-live="assertive"
                        tabIndex={-1}
                      >
                        {formError}
                      </div>
                    )}
                    {/* 用户名 */}
                    <div>
                      <label htmlFor="username" className="auth-label">
                        {t('operatorId')}
                      </label>
                      <div className="relative">
                        <div className="auth-input-icon">
                          <User className="h-4 w-4" />
                        </div>
                        <input
                          ref={usernameInputRef}
                          type="text"
                          id="username"
                          name="username"
                          autoComplete="username"
                          required
                          disabled={loading || isSuccess}
                          value={username}
                          onChange={(e) => {
                            setUsername(e.target.value)
                            setFormError(null)
                          }}
                          placeholder={setupRequired ? 'admin' : t('operatorId')}
                          className="auth-input"
                        />
                      </div>
                    </div>

                    {/* 密码 */}
                    <div>
                      <label htmlFor="password" className="auth-label">
                        {setupRequired ? t('newPassword') : t('password')}
                      </label>
                      <div className="relative">
                        <div className="auth-input-icon">
                          <KeyRound className="h-4 w-4" />
                        </div>
                        <input
                          ref={passwordInputRef}
                          type={showPassword ? 'text' : 'password'}
                          id="password"
                          name="password"
                          autoComplete={setupRequired ? 'new-password' : 'current-password'}
                          required
                          disabled={loading || isSuccess}
                          value={password}
                          onChange={(e) => {
                            setPassword(e.target.value)
                            setFormError(null)
                          }}
                          placeholder="••••••••••••"
                          className="auth-input"
                        />
                        <button
                          type="button"
                          disabled={loading || isSuccess}
                          onClick={() => setShowPassword(!showPassword)}
                          aria-label={showPassword ? t('hidePassword') : t('showPassword')}
                          className="auth-input__toggle"
                        >
                          {showPassword ? (
                            <EyeOff className="h-4 w-4" />
                          ) : (
                            <Eye className="h-4 w-4" />
                          )}
                        </button>
                      </div>
                    </div>

                    {/* 开箱向导模式：确认密码 */}
                    {setupRequired && (
                      <div>
                        <label htmlFor="confirmPassword" className="auth-label">
                          {t('confirmPassword')}
                        </label>
                        <div className="relative">
                          <div className="auth-input-icon">
                            <KeyRound className="h-4 w-4" />
                          </div>
                          <input
                            ref={confirmPasswordInputRef}
                            type={showPassword ? 'text' : 'password'}
                            id="confirmPassword"
                            name="confirmPassword"
                            autoComplete="new-password"
                            required
                            disabled={loading || isSuccess}
                            value={confirmPassword}
                            onChange={(e) => {
                              setConfirmPassword(e.target.value)
                              setFormError(null)
                            }}
                            placeholder="••••••••••••"
                            className="auth-input"
                          />
                        </div>
                      </div>
                    )}

                    {/* 记住凭证选项（仅正常登录） */}
                    {!setupRequired && (
                      <div className="auth-options">
                        <label className="auth-remember">
                          <input
                            type="checkbox"
                            disabled={loading || isSuccess}
                            checked={remember}
                            onChange={(e) => setRemember(e.target.checked)}
                            className="auth-checkbox"
                          />
                          <span>{t('remember')}</span>
                        </label>
                      </div>
                    )}

                    {/* 提交按钮（干脆利落的物理触觉微反馈） */}
                    <div className="pt-2">
                      <button
                        type="submit"
                        disabled={loading || isSuccess}
                        className={`auth-submit ${isSuccess ? 'auth-submit--success' : ''}`}
                      >
                        {isSuccess ? (
                          <>
                            <Check className="h-4 w-4" />
                            <span>{t('loginSuccess')}</span>
                          </>
                        ) : (
                          <>
                            {loading ? (
                              <Loader2 className="h-4 w-4 motion-safe:animate-spin" />
                            ) : (
                              <ArrowRight className="h-4 w-4" />
                            )}
                            <span>{submitLabel}</span>
                          </>
                        )}
                      </button>
                    </div>
                  </>
                )}
              </form>

              <div className="auth-metrics select-none">
                <div className="auth-metric">
                  <div className="auth-metric__label">{t('ingestion')}</div>
                  <div className="auth-metric__value">WebRTC/RTSP</div>
                </div>
                <div className="auth-metric">
                  <div className="auth-metric__label">{t('pipeline')}</div>
                  <div className="auth-metric__value">DMA-BUF</div>
                </div>
                <div className="auth-metric">
                  <div className="auth-metric__label">{t('zeroCopy')}</div>
                  <div className="auth-metric__value auth-metric__value--accent">
                    {t('deviceSide')}
                  </div>
                </div>
              </div>
            </div>

            <footer className="auth-panel__footer">
              <span className="auth-panel__deployment">
                <Server className="h-3.5 w-3.5" />
                <span>{t('deployment')}</span>
              </span>
              <span className="auth-panel__copyright">{t('copyright')}</span>
            </footer>
          </section>
        </div>
      </div>
    </main>
  )
}
