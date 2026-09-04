import React, { useState } from 'react'
import { Eye, EyeOff, KeyRound, Lock, ShieldCheck, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { authApi } from '../lib/api'
import { useAuthStore } from '../stores/auth'

interface ChangePasswordModalProps {
  isOpen: boolean
  onClose: () => void
}

export const ChangePasswordModal: React.FC<ChangePasswordModalProps> = ({ isOpen, onClose }) => {
  const { t } = useTranslation('auth')
  const { username, logout } = useAuthStore()

  const [oldPassword, setOldPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [loading, setLoading] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [successMsg, setSuccessMsg] = useState<string | null>(null)

  if (!isOpen) return null

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

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
      {/* 遮罩背景 */}
      <div
        className="fixed inset-0 bg-black/60 backdrop-blur-sm transition-opacity"
        onClick={handleClose}
      />

      {/* 模态框主体 */}
      <div className="lens-glass relative z-10 w-full max-w-md overflow-hidden rounded-3xl border border-black/10 bg-white/80 p-6 shadow-2xl backdrop-blur-2xl sm:p-7 dark:border-white/10 dark:bg-slate-900/90">
        {/* 顶部标题与关闭按钮 */}
        <div className="flex items-center justify-between border-b border-black/5 pb-4 dark:border-white/5">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-indigo-500/10 text-indigo-500">
              <KeyRound className="h-4 w-4" />
            </div>
            <div>
              <h3 className="font-display text-base font-bold text-slate-900 dark:text-white">
                {t('changePassword')}
              </h3>
              <p className="font-mono text-[10px] text-slate-400">USER // {username || 'admin'}</p>
            </div>
          </div>
          <button
            onClick={handleClose}
            className="rounded-lg p-1.5 text-slate-400 hover:bg-black/5 hover:text-slate-700 dark:hover:bg-white/5 dark:hover:text-slate-200"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 消息提示 */}
        {errorMsg && (
          <div className="mt-4 rounded-xl border border-rose-500/20 bg-rose-500/10 p-3 text-xs text-rose-600 dark:text-rose-400">
            {errorMsg}
          </div>
        )}
        {successMsg && (
          <div className="mt-4 rounded-xl border border-emerald-500/20 bg-emerald-500/10 p-3 text-xs text-emerald-600 dark:text-emerald-400">
            {successMsg}
          </div>
        )}

        {/* 表单 */}
        <form onSubmit={handleSubmit} className="mt-4 space-y-3.5">
          <div>
            <label className="mb-1 block text-xs font-medium text-slate-700 dark:text-slate-300">
              {t('oldPassword')}
            </label>
            <div className="relative">
              <input
                type={showPassword ? 'text' : 'password'}
                required
                value={oldPassword}
                onChange={(e) => setOldPassword(e.target.value)}
                className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-10 pl-3.5 text-sm text-[var(--text-primary)] focus:border-cyan-400 focus:outline-none dark:border-white/10 dark:bg-white/[0.04]"
                placeholder="••••••••"
              />
              <button
                type="button"
                onClick={() => setShowPassword(!showPassword)}
                className="absolute inset-y-0 right-0 flex items-center pr-3 text-slate-400"
              >
                {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </button>
            </div>
          </div>

          <div>
            <label className="mb-1 block text-xs font-medium text-slate-700 dark:text-slate-300">
              {t('newPassword')}
            </label>
            <input
              type={showPassword ? 'text' : 'password'}
              required
              value={newPassword}
              onChange={(e) => setNewPassword(e.target.value)}
              className="w-full rounded-xl border border-black/10 bg-black/[0.03] px-3.5 py-2.5 text-sm text-[var(--text-primary)] focus:border-pink-400 focus:outline-none dark:border-white/10 dark:bg-white/[0.04]"
              placeholder="••••••••"
            />
          </div>

          <div>
            <label className="mb-1 block text-xs font-medium text-slate-700 dark:text-slate-300">
              {t('confirmPassword')}
            </label>
            <input
              type={showPassword ? 'text' : 'password'}
              required
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              className="w-full rounded-xl border border-black/10 bg-black/[0.03] px-3.5 py-2.5 text-sm text-[var(--text-primary)] focus:border-indigo-400 focus:outline-none dark:border-white/10 dark:bg-white/[0.04]"
              placeholder="••••••••"
            />
          </div>

          <div className="flex items-center gap-2 pt-2">
            <button
              type="button"
              onClick={handleClose}
              className="w-1/2 rounded-xl border border-black/10 py-2.5 text-xs font-semibold text-slate-600 hover:bg-black/5 dark:border-white/10 dark:text-slate-300 dark:hover:bg-white/5"
            >
              {t('cancel')}
            </button>
            <button
              type="submit"
              disabled={loading}
              className="font-display flex w-1/2 items-center justify-center gap-1.5 rounded-xl bg-gradient-to-r from-indigo-600 to-cyan-500 py-2.5 text-xs font-bold text-white shadow-md hover:opacity-95 disabled:opacity-50"
            >
              <ShieldCheck className="h-4 w-4" />
              <span>{loading ? t('submittingChange') : t('confirmChange')}</span>
            </button>
          </div>
        </form>

        <div className="mt-4 flex items-center justify-center gap-1.5 font-mono text-[10px] text-slate-400">
          <Lock className="h-3 w-3 text-emerald-500" />
          <span>{t('revokeNotice')}</span>
        </div>
      </div>
    </div>
  )
}
