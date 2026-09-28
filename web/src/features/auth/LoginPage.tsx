import { useCallback, useEffect, useRef, useState } from 'react'
import {
  AlertCircle,
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
import { authApi, RequestTimeoutError } from '@/lib/api'
import { cn } from '@/lib/utils'
import { getRememberedUser, useAuthStore } from '@/stores/auth'
import { toast } from '@/stores/toast'
import { CursorRing } from './components/CursorRing'
import { GargantuaCanvas } from './components/GargantuaCanvas'
import { Starfield } from './components/Starfield'

type InitializationStatus = 'checking' | 'ready' | 'setup-required' | 'unavailable'

/**
 * 把提交失败归一化为可展示的文案。
 *
 * 服务端已按 `Accept-Language` 本地化业务错误，因此优先直接使用其 message；
 * 但传输层故障（超时、断网）没有业务消息，必须回落到本地文案，
 * 否则用户会看到浏览器的原生 abort 提示。
 */
function authErrorMessage(error: unknown, t: (key: string) => string): string {
  if (error instanceof RequestTimeoutError) {
    return t('requestTimeout')
  }
  if (error instanceof TypeError) {
    // fetch 在网络层失败时抛 TypeError（无 message 语义可用）
    return t('networkError')
  }
  if (error instanceof Error && error.message) {
    return error.message
  }
  return t('loginError')
}

export function LoginPage(): React.ReactElement {
  const { t } = useTranslation('auth')
  const login = useAuthStore((state) => state.login)

  const [initializationStatus, setInitializationStatus] = useState<InitializationStatus>('checking')
  const [statusRetryKey, setStatusRetryKey] = useState(0)
  const [username, setUsername] = useState(() => getRememberedUser())
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  // 「记住我」默认不勾选。默认勾选意味着共享终端上任何一次登录都会把操作员 ID
  // 留在下一次打开的页面里；需要跨会话预填的用户主动勾选即可。
  const [remember, setRemember] = useState(false)
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

  // 表单级错误（无法归属到某个输入框）需要把焦点移到错误提示本身，
  // 否则屏幕阅读器用户不会被告知提交失败。错误块是条件渲染的，
  // 因此只能在提交后的 effect 阶段拿到 ref，不能用 setTimeout 兜底。
  const generalErrorFocusRef = useRef(false)
  useEffect(() => {
    if (formError && generalErrorFocusRef.current) {
      generalErrorFocusRef.current = false
      formErrorRef.current?.focus()
    }
  }, [formError])

  const reportGeneralError = (message: string) => {
    generalErrorFocusRef.current = true
    setFormError(message)
  }

  const handleAuthSuccess = (accessToken: string, authUsername: string, expiresAt: number) => {
    setIsSuccess(true)
    // 凭据落盘不走定时器：
    // 1. 路由卸载后仍会执行的 setTimeout 属于游离副作用，且让「登录成功」
    //    与「会话真正生效」之间凭空多出一段无法测试的等待；
    // 2. `loading` 必须在这里复位，否则任何未触发卸载的路径都会把表单
    //    永久锁在「正在校验身份…」，用户既不能重试也看不到错误。
    login(accessToken, authUsername, remember, expiresAt)
    setLoading(false)
  }

  const handleFpsUpdate = useCallback((newFps: number) => {
    const el = document.getElementById('webgl-fps-badge')
    if (el) {
      el.textContent = newFps > 0 ? `${newFps} FPS` : '— FPS'
    }
  }, [])

  useEffect(() => {
    let mounted = true
    // 不在此处同步 setInitializationStatus('checking')：
    // 首次挂载时 useState 初始值已经是 'checking'，重试用下方按钮在事件处理器内重置。
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
    }

    setLoading(true)
    try {
      const credentials = { username: trimmedUsername, password }
      const res = setupRequired
        ? await authApi.initialize(credentials)
        : await authApi.login(credentials)

      if (setupRequired) {
        toast.success(t('setupSuccess'), { title: t('toastSystemReady') })
      }
      handleAuthSuccess(res.accessToken, res.username, res.expiresAt)
    } catch (err: unknown) {
      setLoading(false)
      reportGeneralError(authErrorMessage(err, t))
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

      {/* 核心二元分区架构：左侧 100% 纯净黑洞视界 + 右侧实体深空操作舱 */}
      <div className="pointer-events-none relative z-20 flex min-h-dvh w-full flex-col lg:flex-row">
        {/* 左侧：纯净无界黑洞剧场（完全无遮挡，黑洞自由呼吸） */}
        <div className="flex flex-1 flex-col justify-between p-6 sm:p-10 lg:p-12">
          {/* 左上保留空间对齐 header */}
          <div className="h-10" />

          {/* 中央完全留空给黑洞，没有任何阻碍黑洞的文字卡片 */}
          <div className="flex-1" />

          {/* 左下底栏运行参数状态 */}
          <div className="auth-stage-telemetry pointer-events-auto select-none">
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

        {/* 右侧：实心深空操作舱 (100% Opaque Solid Console，彻底隔绝背景光干扰) */}
        <div className="pointer-events-auto flex w-full flex-shrink-0 flex-col justify-center px-4 py-8 sm:px-6 lg:min-h-dvh lg:w-[440px] lg:px-6 xl:w-[460px] xl:px-8">
          <section
            id="command-dock"
            aria-labelledby="auth-title"
            className="auth-panel mx-auto w-full max-w-[400px] lg:max-w-none"
          >
            <div className="auth-panel__header">
              <div className="auth-panel__status">
                <span className={cn('auth-panel__status-dot', statusView.dotClass)} />
                <span className="auth-panel__status-text">{statusView.badgeText}</span>
              </div>

              <div className="auth-panel__badge">
                <span className="auth-panel__revision">REV. 2026.1</span>
              </div>
            </div>

            <div className="auth-panel__body">
              <div className="auth-panel__intro">
                <div className="auth-panel__eyebrow-row">
                  <span className="auth-panel__eyebrow">{t('terminal')}</span>
                  <span className="auth-panel__serial" aria-hidden="true">
                    SYS://AUTH.GW
                  </span>
                </div>
                <h1 id="auth-title" className="auth-panel__title">
                  {setupRequired && <Wand2 className="auth-setup-icon h-4 w-4" />}
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
                    <div className="flex items-start gap-2.5">
                      <AlertCircle className="auth-form-error__icon h-4 w-4" aria-hidden="true" />
                      <p className="auth-form-error__text flex-1">
                        {t('gatewayUnavailableMessage')}
                      </p>
                    </div>
                    <button
                      type="button"
                      className="auth-retry"
                      onClick={() => {
                        setInitializationStatus('checking')
                        setStatusRetryKey((previous) => previous + 1)
                      }}
                    >
                      <RefreshCw className="mr-2 h-4 w-4" />
                      {t('retryStatus')}
                    </button>
                  </div>
                )}
                {canAuthenticate && (
                  <>
                    {/* 图标与文案必须同处一个横向行内组：.auth-form-error 是
                        column 容器（供可用性分支在其下方追加重试按钮），
                        直接并列子元素会把图标压到文案上方。 */}
                    {formError && (
                      <div
                        ref={formErrorRef}
                        className="auth-form-error"
                        role="alert"
                        aria-live="assertive"
                        tabIndex={-1}
                      >
                        <div className="flex items-start gap-2.5">
                          <AlertCircle
                            className="auth-form-error__icon h-4 w-4"
                            aria-hidden="true"
                          />
                          <span className="auth-form-error__text flex-1">{formError}</span>
                        </div>
                      </div>
                    )}
                    {/* 用户名 */}
                    <div>
                      <label htmlFor="username" className="auth-label">
                        {t('operatorId')}
                      </label>
                      <div className="auth-input-group">
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
                      <div className="auth-input-group">
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
                        <div className="auth-input-group">
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
                          <span className="auth-checkbox">
                            <input
                              type="checkbox"
                              disabled={loading || isSuccess}
                              checked={remember}
                              onChange={(e) => setRemember(e.target.checked)}
                              className="auth-checkbox__native"
                            />
                            <span className="auth-checkbox__box" aria-hidden="true">
                              <Check className="auth-checkbox__check" />
                            </span>
                          </span>
                          <span>{t('remember')}</span>
                        </label>
                      </div>
                    )}

                    {/* 提交按钮（物理触觉高能主电门 + 极客回车指示） */}
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
                            <span className="flex items-center gap-2">
                              {loading ? (
                                <Loader2 className="h-4 w-4 motion-safe:animate-spin" />
                              ) : (
                                <ArrowRight className="h-4 w-4 transition-transform motion-safe:group-hover:translate-x-0.5" />
                              )}
                              <span>{submitLabel}</span>
                            </span>
                            {!loading && (
                              <kbd className="auth-submit__kbd" aria-hidden="true">
                                ↵
                              </kbd>
                            )}
                          </>
                        )}
                      </button>
                    </div>
                  </>
                )}
              </form>

              <div
                className="auth-metrics select-none"
                role="region"
                aria-label="Pipeline Telemetry"
              >
                <div className="auth-metric">
                  <span className="auth-metric__label">{t('ingestion')}</span>
                  <span className="auth-metric__value">WebRTC/RTSP</span>
                </div>
                <div className="auth-metric">
                  <span className="auth-metric__label">{t('pipeline')}</span>
                  <span className="auth-metric__value">DMA-BUF</span>
                </div>
                <div className="auth-metric">
                  <span className="auth-metric__label">{t('zeroCopy')}</span>
                  <span className="auth-metric__value auth-metric__value--accent">
                    {t('deviceSide')}
                  </span>
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
