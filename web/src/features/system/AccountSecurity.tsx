import { useCallback, useEffect, useState } from 'react'
import { Calendar, Clock, KeyRound } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { authApi } from '@/lib/api'
import { formatTimestampShort } from '@/lib/time'
import { RefreshButton } from '@/components/RefreshButton'
import { SettingsSection, LoadingSkeleton, ErrorBanner } from './components/SettingsSection'
import type { AdminUserDto } from '@/types'

interface AccountSecurityProps {
  onOpenPasswordModal?: () => void
}

export function AccountSecurity({ onOpenPasswordModal }: AccountSecurityProps): React.ReactElement {
  const { t, i18n } = useTranslation('system')
  const [user, setUser] = useState<AdminUserDto | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const loadUser = useCallback(
    async (signal?: AbortSignal) => {
      try {
        setLoading(true)
        setError(null)
        const me = await authApi.getMe(signal)
        if (!signal?.aborted) setUser(me)
      } catch (err) {
        if (err instanceof DOMException && err.name === 'AbortError') return
        if (!signal?.aborted)
          setError(
            err instanceof Error
              ? err.message
              : t('loadFailed', { defaultValue: 'Failed to load' }),
          )
      } finally {
        if (!signal?.aborted) setLoading(false)
      }
    },
    [t],
  )

  useEffect(() => {
    const controller = new AbortController()
    loadUser(controller.signal)
    return () => controller.abort()
  }, [loadUser])

  return (
    <div className="space-y-5">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-xl font-bold text-[var(--text-primary)]">
            {t('account.title', { defaultValue: '账号与安全' })}
          </h2>
          <p className="mt-0.5 text-[13px] text-[var(--text-muted)]">
            {t('account.subtitle', { defaultValue: '管理当前管理员账号' })}
          </p>
        </div>
        <RefreshButton onClick={() => loadUser()} loading={loading} />
      </div>

      {error && <ErrorBanner message={error} onRetry={() => loadUser()} />}

      <SettingsSection
        title={t('account.currentAccount', { defaultValue: '当前账号' })}
        action={
          <button
            onClick={onOpenPasswordModal}
            className="flex items-center gap-2 rounded-xl bg-[var(--accent)] px-4 py-2 text-[13px] font-medium text-white shadow-[var(--accent)]/20 shadow-lg transition-all hover:bg-[var(--accent)]/90 active:scale-[0.97]"
          >
            <KeyRound className="h-3.5 w-3.5" />
            {t('account.changePassword', { defaultValue: '修改密码' })}
          </button>
        }
      >
        {loading && !user ? (
          <LoadingSkeleton rows={3} />
        ) : user ? (
          <div className="space-y-3.5">
            {/* 核心身份主卡片 */}
            <div className="frosted-glass relative flex items-center justify-between gap-4 overflow-hidden rounded-2xl border border-[var(--border)] p-4 shadow-sm">
              <div className="pointer-events-none absolute top-0 right-0 left-0 h-[1.5px] bg-gradient-to-r from-emerald-500/0 via-emerald-400/80 to-emerald-500/0 opacity-70" />
              <div className="flex items-center gap-4">
                <div className="relative flex h-14 w-14 shrink-0 items-center justify-center rounded-2xl border border-[var(--border)]/80 bg-gradient-to-b from-[var(--accent-soft)] to-[var(--bg-secondary)] shadow-inner">
                  <span className="font-mono text-xl font-black text-[var(--accent)]">
                    {user.username ? user.username.charAt(0).toUpperCase() : 'A'}
                  </span>
                  <span
                    aria-hidden="true"
                    className="absolute -right-0.5 -bottom-0.5 h-3.5 w-3.5 rounded-full border-2 border-[var(--bg-surface-solid)] bg-emerald-500 shadow-xs"
                  />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <p className="font-mono text-lg font-bold tracking-tight text-[var(--text-primary)]">
                      {user.username}
                    </p>
                    <span className="inline-flex items-center gap-1 rounded-md border border-indigo-500/25 bg-indigo-500/10 px-2 py-0.5 text-[10px] font-semibold text-indigo-400">
                      ROOT
                    </span>
                  </div>
                  <div className="mt-1 flex items-center gap-2 text-xs text-[var(--text-muted)]">
                    <span className="flex items-center gap-1.5">
                      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-emerald-400" />
                      <span className="font-medium text-emerald-400/90">
                        {t('account.activeSession', { defaultValue: 'Active Session' })}
                      </span>
                    </span>
                  </div>
                </div>
              </div>
            </div>

            <div className="grid grid-cols-2 gap-3">
              <div className="flex items-center gap-3 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 p-3.5 shadow-2xs">
                <Calendar className="h-4 w-4 shrink-0 text-[var(--text-muted)]" />
                <div className="min-w-0">
                  <p className="text-[10px] font-medium tracking-wider text-[var(--text-muted)] uppercase">
                    {t('account.createdAt', { defaultValue: '创建时间' })}
                  </p>
                  <p className="font-data mt-0.5 truncate text-[13px] font-semibold text-[var(--text-primary)] tabular-nums">
                    {formatTimestampShort(user.createdAt, i18n.language)}
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 p-3.5 shadow-2xs">
                <Clock className="h-4 w-4 shrink-0 text-[var(--text-muted)]" />
                <div className="min-w-0">
                  <p className="text-[10px] font-medium tracking-wider text-[var(--text-muted)] uppercase">
                    {t('account.updatedAt', { defaultValue: '更新时间' })}
                  </p>
                  <p className="font-data mt-0.5 truncate text-[13px] font-semibold text-[var(--text-primary)] tabular-nums">
                    {formatTimestampShort(user.updatedAt, i18n.language)}
                  </p>
                </div>
              </div>
            </div>
          </div>
        ) : null}
      </SettingsSection>
    </div>
  )
}
