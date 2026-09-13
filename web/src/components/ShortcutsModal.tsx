import React from 'react'
import { Compass, CornerDownLeft, Keyboard, Layers, Search, Sliders, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '../hooks/use-dismiss-stack'

export interface ShortcutsModalProps {
  isOpen: boolean
  onClose: () => void
}

interface ShortcutItem {
  keys: string[]
  descKey: string
  detail?: string
}

interface ShortcutCategory {
  categoryKey: string
  icon: typeof Compass
  items: ShortcutItem[]
}

const SHORTCUT_GROUPS: ShortcutCategory[] = [
  {
    categoryKey: 'shortcuts.categories.navigation',
    icon: Compass,
    items: [
      {
        keys: ['1', '…', '8'],
        descKey: 'shortcuts.items.navTabs',
        detail: 'Alt + 1 ~ 8',
      },
      {
        keys: ['T'],
        descKey: 'shortcuts.items.toggleTheme',
        detail: 'Alt + T',
      },
    ],
  },
  {
    categoryKey: 'shortcuts.categories.general',
    icon: Sliders,
    items: [
      {
        keys: ['?'],
        descKey: 'shortcuts.items.help',
        detail: 'Shift + /',
      },
      {
        keys: ['/'],
        descKey: 'shortcuts.items.quickSearch',
      },
    ],
  },
  {
    categoryKey: 'shortcuts.categories.modals',
    icon: Layers,
    items: [
      {
        keys: ['Esc'],
        descKey: 'shortcuts.items.escDismiss',
      },
      {
        keys: ['Enter'],
        descKey: 'shortcuts.items.enterConfirm',
      },
    ],
  },
]

export const ShortcutsModal: React.FC<ShortcutsModalProps> = ({ isOpen, onClose }) => {
  const { t } = useTranslation('common')

  // 接入浮层栈：按 Esc 时优先关闭快捷键面板
  useDismissStack(isOpen, onClose, { priority: 20 })

  if (!isOpen) return null

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('shortcuts.title')}
      className="animate-in fade-in fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-xs duration-200"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="frosted-glass relative flex max-h-[85vh] w-full max-w-2xl flex-col overflow-hidden rounded-3xl border border-[var(--border)] bg-[var(--bg-surface-solid)] shadow-2xl">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-[var(--border)] px-6 py-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-2xl bg-[var(--accent-soft)] text-[var(--accent)]">
              <Keyboard className="h-5 w-5" />
            </div>
            <div>
              <h3 className="text-base font-bold text-[var(--text-primary)]">
                {t('shortcuts.title')}
              </h3>
              <p className="text-xs text-[var(--text-muted)]">{t('shortcuts.subtitle')}</p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t('close')}
            className="rounded-xl p-2 text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Content Body */}
        <div className="flex-1 space-y-6 overflow-y-auto p-6">
          {SHORTCUT_GROUPS.map((group) => {
            const GroupIcon = group.icon
            return (
              <div key={group.categoryKey} className="space-y-3">
                <div className="flex items-center gap-2 text-xs font-semibold tracking-wider text-[var(--text-muted)] uppercase">
                  <GroupIcon className="h-4 w-4 text-[var(--accent)]" />
                  <span>{t(group.categoryKey)}</span>
                </div>

                <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
                  {group.items.map((item, idx) => (
                    <div
                      key={idx}
                      className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-3.5 py-2.5 transition-colors hover:border-[var(--accent-soft)]"
                    >
                      <div className="space-y-0.5">
                        <span className="text-xs font-medium text-[var(--text-primary)]">
                          {t(item.descKey)}
                        </span>
                        {item.detail && (
                          <p className="font-mono text-[10px] text-[var(--text-muted)]">
                            {item.detail}
                          </p>
                        )}
                      </div>
                      <div className="flex items-center gap-1">
                        {item.keys.map((k, kIdx) => (
                          <kbd
                            key={kIdx}
                            className="inline-flex min-h-[22px] min-w-[22px] items-center justify-center rounded-md border border-[var(--border)] bg-[var(--bg-surface)] px-1.5 font-mono text-[11px] font-semibold text-[var(--text-primary)] shadow-xs"
                          >
                            {k === 'Enter' ? (
                              <CornerDownLeft className="h-3 w-3" />
                            ) : k === 'Search' ? (
                              <Search className="h-3 w-3" />
                            ) : (
                              k
                            )}
                          </kbd>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )
          })}
        </div>

        {/* Footer Hint */}
        <div className="flex items-center justify-between border-t border-[var(--border)] bg-[var(--bg-secondary)] px-6 py-3 text-xs text-[var(--text-muted)]">
          <span>{t('shortcuts.hint')}</span>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1 text-xs font-medium text-[var(--text-primary)] transition-colors hover:bg-[var(--surface-hover)]"
          >
            {t('close')}
          </button>
        </div>
      </div>
    </div>
  )
}
