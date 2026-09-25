/**
 * 账号面板抽屉（用户域唯一入口）
 *
 * 归属 `features/auth`，与 `system` 设备域彻底分离：设置页只保留网络/存储/对时/国标
 * 等跟随设备走的配置，账号身份与改密跟随使用者走。
 *
 * 外壳复用 `ModalOverlay variant="drawer"`，由此获得焦点陷阱与关闭后的焦点归还，
 * 不在菜单里自造 `role="menu"` 语义（声明了方向键契约却无实现比不声明更糟）。
 */

import { useCallback, useEffect, useState, type ReactElement } from 'react'
import {
  Calendar,
  Clock,
  KeyRound,
  Loader2,
  RefreshCw,
  ShieldCheck,
  UserCog,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ModalOverlay } from '@/components/ui/ModalOverlay'
import { authApi } from '@/lib/api'
import { formatTimestampShort } from '@/lib/time'
import type { AdminUserDto } from '@/types'

export interface AccountPanelDrawerProps {
  isOpen: boolean
  onClose: () => void
  onOpenPasswordModal: () => void
}

export function AccountPanelDrawer({
  isOpen,
  onClose,
  onOpenPasswordModal,
}: AccountPanelDrawerProps): ReactElement {
  const { t, i18n } = useTranslation('auth')
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
        if (!signal?.aborted) {
          setError(err instanceof Error ? err.message : t('loadFailed'))
        }
      } finally {
        if (!signal?.aborted) setLoading(false)
      }
    },
    [t],
  )

  useEffect(() => {
    if (!isOpen) return
    const controller = new AbortController()
    void loadUser(controller.signal)
    return () => controller.abort()
  }, [isOpen, loadUser])

  return (
    <ModalOverlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabel={t('accountPanel.title')}
      variant="drawer"
      panelClassName="modal-surface--drawer-medium"
    >
      <header className="flex shrink-0 items-center justify-between border-b border-[var(--border)] pb-4">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <UserCog className="h-4 w-4" aria-hidden="true" />
          </div>
          <div className="min-w-0">
            <h2 className="truncate text-sm font-semibold text-[var(--text-primary)]">
              {t('accountPanel.title')}
            </h2>
            <p className="mt-0.5 text-[11px] text-[var(--text-muted)]">
              {t('accountPanel.subtitle')}
            </p>
          </div>
        </div>
        <button
          type="button"
          data-autofocus
          onClick={onClose}
          aria-label={t('close', { ns: 'common', defaultValue: '关闭' })}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
        >
          <X className="h-4 w-4" aria-hidden="true" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto py-5">
        {error && (
          <div
            role="alert"
            className="mb-4 flex items-start gap-2.5 rounded-xl border border-[var(--status-danger)]/20 bg-[var(--status-danger-soft)] p-3 text-xs text-[var(--status-danger)]"
          >
            <div className="min-w-0 flex-1 leading-relaxed">{error}</div>
            <button
              type="button"
              onClick={() => void loadUser()}
              className="inline-flex shrink-0 items-center gap-1.5 font-medium hover:underline focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
            >
              <RefreshCw className="h-3 w-3" aria-hidden="true" />
              <span>{t('accountPanel.retry')}</span>
            </button>
          </div>
        )}

        {loading && !user ? (
          <div
            role="status"
            className="flex items-center gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3 text-xs text-[var(--text-secondary)]"
          >
            <Loader2
              className="h-4 w-4 shrink-0 animate-spin text-[var(--accent)] motion-reduce:animate-none"
              aria-hidden="true"
            />
            <span>{t('accountPanel.loading')}</span>
          </div>
        ) : user ? (
          <div className="space-y-4">
            {/* 身份主卡：角色文案统一走 auth:roleAdministrator，不再有硬编码 ROOT */}
            <div className="relative overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)]/40 p-4 shadow-xs">
              <div className="flex items-center gap-3.5">
                <div className="relative flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl border border-[var(--border)]/80 bg-gradient-to-b from-[var(--accent-soft)] to-[var(--bg-secondary)] shadow-inner">
                  <span className="font-mono text-lg font-black text-[var(--accent)]">
                    {user.username ? user.username.charAt(0).toUpperCase() : 'A'}
                  </span>
                  <span
                    aria-hidden="true"
                    className="absolute -right-0.5 -bottom-0.5 h-3 w-3 rounded-full border-2 border-[var(--bg-surface-solid)] bg-emerald-500"
                  />
                </div>
                <div className="min-w-0">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="truncate font-mono text-base font-bold tracking-tight text-[var(--text-primary)]">
                      {user.username}
                    </p>
                    <span className="inline-flex items-center gap-1 rounded-md border border-[var(--accent)]/25 bg-[var(--accent-soft)] px-2 py-0.5 text-[10px] font-semibold text-[var(--accent)]">
                      <ShieldCheck className="h-3 w-3" aria-hidden="true" />
                      {t('roleAdministrator')}
                    </span>
                  </div>
                  <p className="mt-1 text-[11px] text-[var(--text-muted)]">
                    {t('accountPanel.activeSession')}
                  </p>
                </div>
              </div>
            </div>

            {/* 时间元数据：两列并排，窄视口自动堆叠 */}
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <div className="flex items-center gap-3 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 p-3.5 shadow-2xs">
                <Calendar
                  className="h-4 w-4 shrink-0 text-[var(--text-muted)]"
                  aria-hidden="true"
                />
                <div className="min-w-0">
                  <p className="text-[10px] font-medium tracking-wider text-[var(--text-muted)] uppercase">
                    {t('accountPanel.createdAt')}
                  </p>
                  <p className="font-data mt-0.5 truncate text-[13px] font-semibold text-[var(--text-primary)] tabular-nums">
                    {formatTimestampShort(user.createdAt, i18n.language)}
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-3 rounded-xl border border-[var(--border)]/60 bg-[var(--bg-secondary)]/40 p-3.5 shadow-2xs">
                <Clock className="h-4 w-4 shrink-0 text-[var(--text-muted)]" aria-hidden="true" />
                <div className="min-w-0">
                  <p className="text-[10px] font-medium tracking-wider text-[var(--text-muted)] uppercase">
                    {t('accountPanel.updatedAt')}
                  </p>
                  <p className="font-data mt-0.5 truncate text-[13px] font-semibold text-[var(--text-primary)] tabular-nums">
                    {formatTimestampShort(user.updatedAt, i18n.language)}
                  </p>
                </div>
              </div>
            </div>
          </div>
        ) : null}

        {/*
         * 改密入口不依赖 getMe 结果：账号元数据拉取失败或慢网络下，
         * 用户仍必须能修改密码。主操作放在数据依赖之外。
         */}
        <div className={user || loading ? 'mt-4' : ''}>
          <button
            type="button"
            onClick={onOpenPasswordModal}
            className="flex w-full items-center justify-center gap-2 rounded-xl bg-[var(--accent)] px-4 py-2.5 text-[13px] font-medium text-white shadow-[var(--accent)]/20 shadow-lg transition-all hover:bg-[var(--accent)]/90 focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden active:scale-[0.98]"
          >
            <KeyRound className="h-3.5 w-3.5" aria-hidden="true" />
            {t('changePassword')}
          </button>
          <p className="mt-2.5 text-center text-[11px] leading-relaxed text-[var(--text-muted)]">
            {t('revokeNotice')}
          </p>
        </div>
      </div>
    </ModalOverlay>
  )
}
