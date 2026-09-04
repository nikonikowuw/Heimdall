import React, { useState } from 'react'
import { ArrowRight, KeyRound, ShieldCheck, User, Wand2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { authApi } from '../../lib/api'
import { useAuthStore } from '../../stores/auth'

interface SetupPageProps {
  onSuccess?: () => void
}

export const SetupPage: React.FC<SetupPageProps> = ({ onSuccess }) => {
  const { t } = useTranslation('auth')
  const login = useAuthStore((state) => state.login)

  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [loading, setLoading] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    setErrorMsg(null)

    const trimmedUsername = username.trim()
    if (!trimmedUsername || !password) {
      setErrorMsg(t('loginError'))
      return
    }

    if (password.length < 6) {
      setErrorMsg(t('passwordLengthError'))
      return
    }

    if (password !== confirmPassword) {
      setErrorMsg(t('passwordMismatch'))
      return
    }

    setLoading(true)
    try {
      const res = await authApi.initialize({
        username: trimmedUsername,
        password,
      })
      login(res.accessToken, res.username, true)
      if (onSuccess) {
        onSuccess()
      }
    } catch (err: unknown) {
      setLoading(false)
      const msg = err instanceof Error ? err.message : t('loginError')
      setErrorMsg(msg)
    }
  }

  return (
    <div className="flex min-h-full w-full flex-col justify-center">
      <div className="mb-5 space-y-1">
        <h2 className="font-display flex items-center gap-2 text-2xl font-bold tracking-tight text-slate-900 dark:text-white">
          <Wand2 className="h-5 w-5 text-amber-500" />
          <span>{t('setupTitle')}</span>
        </h2>
        <p className="text-xs leading-relaxed text-slate-500 dark:text-slate-400">
          {t('setupSubtitle')}
        </p>
      </div>

      {errorMsg && (
        <div className="mb-4 rounded-xl border border-rose-500/20 bg-rose-500/10 p-3 text-xs text-rose-600 dark:text-rose-400">
          {errorMsg}
        </div>
      )}

      <form onSubmit={handleSubmit} className="space-y-4">
        <div>
          <label
            htmlFor="setup-username"
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
              id="setup-username"
              required
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              placeholder="admin"
              className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-3.5 pl-10 text-sm text-[var(--text-primary)] focus:border-indigo-400 focus:outline-none dark:border-white/10 dark:bg-white/[0.04]"
            />
          </div>
        </div>

        <div>
          <label
            htmlFor="setup-password"
            className="mb-1.5 block text-xs font-medium text-slate-700 dark:text-slate-300"
          >
            {t('newPassword')}
          </label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3.5 text-slate-400">
              <KeyRound className="h-4 w-4" />
            </div>
            <input
              type="password"
              id="setup-password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder="••••••••••••"
              className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-3.5 pl-10 text-sm text-[var(--text-primary)] focus:border-indigo-400 focus:outline-none dark:border-white/10 dark:bg-white/[0.04]"
            />
          </div>
        </div>

        <div>
          <label
            htmlFor="setup-confirm-password"
            className="mb-1.5 block text-xs font-medium text-slate-700 dark:text-slate-300"
          >
            {t('confirmPassword')}
          </label>
          <div className="relative">
            <div className="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3.5 text-slate-400">
              <ShieldCheck className="h-4 w-4" />
            </div>
            <input
              type="password"
              id="setup-confirm-password"
              required
              value={confirmPassword}
              onChange={(e) => setConfirmPassword(e.target.value)}
              placeholder="••••••••••••"
              className="w-full rounded-xl border border-black/10 bg-black/[0.03] py-2.5 pr-3.5 pl-10 text-sm text-[var(--text-primary)] focus:border-indigo-400 focus:outline-none dark:border-white/10 dark:bg-white/[0.04]"
            />
          </div>
        </div>

        <button
          type="submit"
          disabled={loading}
          className="font-display flex w-full cursor-pointer items-center justify-center gap-2 rounded-xl bg-gradient-to-r from-amber-500 via-indigo-600 to-cyan-500 px-4 py-3 text-xs font-bold tracking-wider text-white uppercase shadow-lg shadow-indigo-600/25 transition-all duration-200 hover:opacity-95 active:scale-[0.99] disabled:opacity-50"
        >
          <ArrowRight className={`h-4 w-4 ${loading ? 'animate-spin' : ''}`} />
          <span>{loading ? t('submitting') : t('setupSubmit')}</span>
        </button>
      </form>
    </div>
  )
}
