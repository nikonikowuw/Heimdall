import { useState } from 'react'
import { motion, AnimatePresence } from 'motion/react'
import { useTranslation } from 'react-i18next'
import type { LucideIcon } from 'lucide-react'
import { Settings, User, Wifi, HardDrive, Clock, LayoutDashboard, ChevronRight } from 'lucide-react'
import { SystemOverview } from './SystemOverview'
import { AccountSecurity } from './AccountSecurity'
import { NetworkSettings } from './NetworkSettings'
import { StorageSettings } from './StorageSettings'
import { TimeSettings } from './TimeSettings'

type SettingsTab = 'overview' | 'account' | 'network' | 'storage' | 'time'

const TABS: {
  key: SettingsTab
  icon: LucideIcon
  labelKey: string
  descKey: string
}[] = [
  {
    key: 'overview',
    icon: LayoutDashboard,
    labelKey: 'tabs.overview',
    descKey: 'tabs.overviewDesc',
  },
  { key: 'account', icon: User, labelKey: 'tabs.account', descKey: 'tabs.accountDesc' },
  { key: 'network', icon: Wifi, labelKey: 'tabs.network', descKey: 'tabs.networkDesc' },
  { key: 'storage', icon: HardDrive, labelKey: 'tabs.storage', descKey: 'tabs.storageDesc' },
  { key: 'time', icon: Clock, labelKey: 'tabs.time', descKey: 'tabs.timeDesc' },
]

interface SettingsPageProps {
  onOpenPasswordModal?: () => void
}

export function SettingsPage({ onOpenPasswordModal }: SettingsPageProps): React.ReactElement {
  const { t } = useTranslation('system')
  const [activeTab, setActiveTab] = useState<SettingsTab>('overview')

  return (
    <div className="flex h-full gap-5 overflow-hidden">
      {/* Sidebar navigation */}
      <aside className="frosted-glass flex w-56 shrink-0 flex-col rounded-2xl p-3">
        <div className="mb-4 flex items-center gap-2.5 px-1">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-[var(--accent)]/10">
            <Settings className="h-4 w-4 text-[var(--accent)]" />
          </div>
          <span className="text-sm font-semibold tracking-tight text-[var(--text-primary)]">
            {t('title', { defaultValue: '系统设置' })}
          </span>
        </div>

        <nav className="mt-1 flex flex-col gap-1">
          {TABS.map(({ key, icon: Icon, labelKey, descKey }) => {
            const isActive = activeTab === key
            return (
              <motion.button
                key={key}
                onClick={() => setActiveTab(key)}
                whileHover={{ scale: 1.02 }}
                whileTap={{ scale: 0.97 }}
                transition={{ duration: 0.15, ease: [0.22, 1, 0.36, 1] }}
                className={`group relative flex items-center gap-3 rounded-xl px-3.5 py-3 text-left transition-all duration-200 ${
                  isActive
                    ? 'bg-[var(--accent)] text-white shadow-[var(--accent)]/25 shadow-lg'
                    : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]'
                }`}
              >
                <Icon
                  className={`h-4.5 w-4.5 shrink-0 ${isActive ? 'text-white' : 'text-[var(--text-muted)] group-hover:text-[var(--accent)]'}`}
                />
                <div className="min-w-0 flex-1">
                  <div className="text-sm leading-tight font-medium">
                    {t(labelKey, { defaultValue: key })}
                  </div>
                  <div
                    className={`mt-0.5 text-[11px] leading-tight ${isActive ? 'text-white/70' : 'text-[var(--text-muted)]'}`}
                  >
                    {t(descKey, { defaultValue: '' })}
                  </div>
                </div>
                <ChevronRight
                  className={`h-3.5 w-3.5 shrink-0 transition-transform ${isActive ? 'translate-x-0 text-white/70' : '-translate-x-1 text-[var(--text-muted)] opacity-0 group-hover:translate-x-0 group-hover:opacity-100'}`}
                />
              </motion.button>
            )
          })}
        </nav>
      </aside>

      {/* Content area */}
      <main className="flex-1 overflow-y-auto pr-1">
        <div className="pb-8">
          <AnimatePresence mode="wait">
            <motion.div
              key={activeTab}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
            >
              {activeTab === 'overview' && <SystemOverview />}
              {activeTab === 'account' && (
                <AccountSecurity onOpenPasswordModal={onOpenPasswordModal} />
              )}
              {activeTab === 'network' && <NetworkSettings />}
              {activeTab === 'storage' && <StorageSettings />}
              {activeTab === 'time' && <TimeSettings />}
            </motion.div>
          </AnimatePresence>
        </div>
      </main>
    </div>
  )
}
