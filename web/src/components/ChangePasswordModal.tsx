import { useState } from 'react'
import { Eye, EyeOff, KeyRound, Loader2, Lock, ShieldCheck, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { authApi } from '@/lib/api'
import { useAuthStore } from '@/stores/auth'

export interface ChangePasswordModalProps {
  isOpen: boolean
  onClose: () => void
}

export function ChangePasswordModal({ isOpen, onClose }: ChangePasswordModalProps) {
  const { t } = useTranslation('auth')
  const { username, logout } = useAuthStore()

  const [oldPassword, setOldPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [loading, setLoading] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [successMsg, setSuccessMsg] = useState<string | null>(null)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setErrorMsg(null)
    setSuccessMsg(null)

    if (!oldPassword || !newPassword || !confirmPassword) {
      setErrorMsg(t('loginError'))
      return
    }

    if (newPassword.length < 6) {
      setErrorMsg(t('newPassword'))
      return
    }

    if (newPassword !== confirmPassword) {
      setErrorMsg(t('passwordMismatch'))
      return
    }

    setLoading(true)
    try {
      await authApi.changePassword({ oldPassword, newPassword })
      setSuccessMsg(t('passwordChanged'))
      setTimeout(() => {
        setOldPassword('')
        setNewPassword('')
        setConfirmPassword('')
        setErrorMsg(null)
        setSuccessMsg(null)
        onClose()
        logout()
      }, 1500)
    } catch (err: unknown) {
      setLoading(false)
      const msg = err instanceof Error ? err.message : t('loginError')
      setErrorMsg(msg)
    }
  }

  const handleClose = () => {
    setOldPassword('')
    setNewPassword('')
    setConfirmPassword('')
    setErrorMsg(null)
    setSuccessMsg(null)
    onClose()
  }

  useDismissStack(isOpen, handleClose, { disabled: loading })

  return (
    <AnimatePresence>
      {isOpen && (
        <div
          onClick={(e) => {
            if (e.target === e.currentTarget && !loading) {
              handleClose()
            }
          }}
          className="fixed inset-0 z-[70] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="relative flex w-full max-w-md flex-col overflow-hidden rounded-[26px] border border-[var(--border)] bg-white shadow-[0_24px_50px_-12px_rgba(0,0,0,0.28)] dark:bg-[var(--bg-surface-solid)]"
          >
            {/* 顶部标题与关闭按钮 */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-6 py-4.5">
              <div className="flex items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs">
                  <KeyRound className="h-5 w-5" />
                </div>
                <div>
                  <h3 className="text-base font-bold tracking-tight text-[var(--text-primary)]">
                    {t('changePassword')}
                  </h3>
                  <p className="font-data mt-0.5 text-[11px] text-[var(--text-muted)]">
                    USER // {username || 'admin'}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={handleClose}
                disabled={loading}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none disabled:opacity-50"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* 消息提示 */}
            <div className="space-y-3.5 p-6">
              {errorMsg && (
                <div className="rounded-2xl border border-rose-500/20 bg-rose-500/10 p-3.5 text-xs text-rose-500">
                  {errorMsg}
                </div>
              )}
              {successMsg && (
                <div className="rounded-2xl border border-emerald-500/20 bg-emerald-500/10 p-3.5 text-xs text-emerald-600 dark:text-emerald-400">
                  {successMsg}
                </div>
              )}

              {/* 表单 */}
              <form onSubmit={handleSubmit} className="space-y-3.5">
                <div>
                  <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                    {t('oldPassword')}
                  </label>
                  <div className="relative mt-1.5">
                    <input
                      type={showPassword ? 'text' : 'password'}
                      required
                      value={oldPassword}
                      onChange={(e) => setOldPassword(e.target.value)}
                      className="font-data w-full rounded-xl border border-[var(--border)]/80 bg-white py-2 pr-10 pl-3.5 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-all focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/15 focus:outline-none dark:bg-[var(--bg-surface-solid)]"
                      placeholder="••••••••"
                    />
                    <button
                      type="button"
                      onClick={() => setShowPassword(!showPassword)}
                      className="absolute inset-y-0 right-0 flex items-center pr-3 text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                    >
                      {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                    </button>
                  </div>
                </div>

                <div>
                  <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                    {t('newPassword')}
                  </label>
                  <input
                    type={showPassword ? 'text' : 'password'}
                    required
                    value={newPassword}
                    onChange={(e) => setNewPassword(e.target.value)}
                    className="font-data mt-1.5 w-full rounded-xl border border-[var(--border)]/80 bg-white px-3.5 py-2 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-all focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/15 focus:outline-none dark:bg-[var(--bg-surface-solid)]"
                    placeholder="••••••••"
                  />
                </div>

                <div>
                  <label className="block text-xs font-semibold text-[var(--text-secondary)]">
                    {t('confirmPassword')}
                  </label>
                  <input
                    type={showPassword ? 'text' : 'password'}
                    required
                    value={confirmPassword}
                    onChange={(e) => setConfirmPassword(e.target.value)}
                    className="font-data mt-1.5 w-full rounded-xl border border-[var(--border)]/80 bg-white px-3.5 py-2 text-xs text-[var(--text-primary)] placeholder-[var(--text-muted)] transition-all focus:border-[var(--accent)] focus:ring-2 focus:ring-[var(--accent)]/15 focus:outline-none dark:bg-[var(--bg-surface-solid)]"
                    placeholder="••••••••"
                  />
                </div>

                <div className="flex items-center gap-2 pt-2">
                  <button
                    type="button"
                    onClick={handleClose}
                    disabled={loading}
                    className="w-1/2 rounded-xl border border-[var(--border)]/80 bg-white py-2 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] disabled:opacity-50 dark:bg-[var(--bg-surface-solid)]"
                  >
                    {t('cancel')}
                  </button>
                  <button
                    type="submit"
                    disabled={loading}
                    className="flex w-1/2 items-center justify-center gap-1.5 rounded-xl bg-slate-900 py-2 text-xs font-semibold text-white shadow-xs transition-all hover:bg-slate-800 active:scale-95 disabled:opacity-50 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white"
                  >
                    {loading ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <ShieldCheck className="h-4 w-4" />
                    )}
                    <span>{loading ? t('submittingChange') : t('confirmChange')}</span>
                  </button>
                </div>
              </form>

              <div className="flex items-center justify-center gap-1.5 pt-1 text-[11px] text-[var(--text-muted)]">
                <Lock className="h-3 w-3 text-emerald-500" />
                <span>{t('revokeNotice')}</span>
              </div>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
