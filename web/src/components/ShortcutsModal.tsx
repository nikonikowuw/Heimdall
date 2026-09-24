import React from 'react'
import { Command, CornerDownLeft, Grid, Keyboard, Layers, Search, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'

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
  icon: React.ElementType
  items: ShortcutItem[]
}

const SHORTCUT_GROUPS: ShortcutCategory[] = [
  {
    categoryKey: 'shortcuts.categories.navigation',
    icon: Grid,
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
    icon: Command,
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

function renderKeyCap(key: string): React.ReactNode {
  if (key === 'Enter') {
    return <CornerDownLeft className="h-3 w-3" />
  }
  if (key === 'Search') {
    return <Search className="h-3 w-3" />
  }
  return key
}

export function ShortcutsModal({ isOpen, onClose }: ShortcutsModalProps): React.ReactElement {
  const { t } = useTranslation('common')

  // 接入浮层栈：按 Esc 时优先关闭快捷键面板
  useDismissStack(isOpen, onClose, { priority: 20 })

  return (
    <AnimatePresence>
      {isOpen && (
        <div
          role="dialog"
          aria-modal="true"
          aria-label={t('shortcuts.title')}
          className="modal-backdrop modal-backdrop--top"
          onClick={(e) => {
            if (e.target === e.currentTarget) onClose()
          }}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 10 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 10 }}
            transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
            className="modal-surface modal-surface--medium max-h-[85vh]"
          >
            {/* Header */}
            <div className="flex shrink-0 items-center justify-between border-b border-[var(--border)]/70 px-6 py-4.5">
              <div className="flex items-center gap-3.5">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl border border-[var(--accent)]/20 bg-[var(--accent-soft)] text-[var(--accent)] shadow-xs">
                  <Keyboard className="h-5 w-5" />
                </div>
                <div>
                  <h3 className="text-base font-bold tracking-tight text-[var(--text-primary)] sm:text-lg">
                    {t('shortcuts.title')}
                  </h3>
                  <p className="mt-0.5 text-xs text-[var(--text-muted)]">
                    {t('shortcuts.subtitle')}
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={onClose}
                aria-label={t('close')}
                className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-none"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            {/* Content Body */}
            <div className="flex-1 space-y-5 overflow-y-auto p-6">
              {SHORTCUT_GROUPS.map((group) => {
                const GroupIcon = group.icon
                return (
                  <div key={group.categoryKey} className="space-y-2.5">
                    <div className="flex items-center gap-2 text-xs font-bold tracking-wider text-[var(--text-muted)] uppercase">
                      <GroupIcon className="h-3.5 w-3.5 text-[var(--accent)]" />
                      <span>{t(group.categoryKey)}</span>
                    </div>

                    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                      {group.items.map((item, idx) => (
                        <div
                          key={idx}
                          className="flex items-center justify-between rounded-xl border border-[var(--border)]/70 bg-[var(--bg-secondary)]/25 px-3.5 py-2.5 transition-colors hover:border-[var(--accent)]/40 hover:bg-white dark:hover:bg-[var(--bg-surface-solid)]"
                        >
                          <div className="space-y-0.5">
                            <span className="text-xs font-medium text-[var(--text-primary)]">
                              {t(item.descKey)}
                            </span>
                            {item.detail && (
                              <p className="font-data text-[10px] text-[var(--text-muted)]">
                                {item.detail}
                              </p>
                            )}
                          </div>
                          <div className="flex items-center gap-1">
                            {item.keys.map((k, kIdx) => (
                              <kbd
                                key={kIdx}
                                className="font-data inline-flex min-h-[22px] min-w-[22px] items-center justify-center rounded-lg border border-[var(--border)] bg-white px-1.5 text-[11px] font-semibold text-[var(--text-primary)] shadow-2xs dark:bg-[var(--bg-surface-solid)]"
                              >
                                {renderKeyCap(k)}
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
            <div className="flex shrink-0 items-center justify-between border-t border-[var(--border)]/70 bg-[var(--bg-secondary)]/20 px-6 py-3.5 text-xs text-[var(--text-muted)]">
              <span>{t('shortcuts.hint')}</span>
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl border border-[var(--border)]/80 bg-white px-3.5 py-1.5 text-xs font-medium text-[var(--text-secondary)] shadow-xs transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)] dark:bg-[var(--bg-surface-solid)]"
              >
                {t('close')}
              </button>
            </div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}
