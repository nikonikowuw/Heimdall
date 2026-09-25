import { useState, type ReactElement } from 'react'
import { Activity, ShieldCheck } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { PageHeader } from '@/components/ui/PageHeader'
import { OperationLogsTab } from './OperationLogsTab'
import { OperationalLogsTab } from './OperationalLogsTab'

type LogTab = 'operations' | 'operational'

interface TabOption {
  id: LogTab
  labelKey: 'tabs.operations' | 'tabs.operational'
  icon: typeof ShieldCheck
}

const TABS: readonly TabOption[] = [
  { id: 'operations', labelKey: 'tabs.operations', icon: ShieldCheck },
  { id: 'operational', labelKey: 'tabs.operational', icon: Activity },
] as const

export function OplogPage(): ReactElement {
  const { t } = useTranslation('oplog')
  const [activeTab, setActiveTab] = useState<LogTab>('operations')
  const reducedMotion = useReducedMotion()

  const ActiveIcon = activeTab === 'operations' ? ShieldCheck : Activity

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 text-[var(--text-primary)] select-none">
      {/* 顶部日志中心统一导航条 */}
      <PageHeader
        icon={ActiveIcon}
        iconIndicator={
          <span className="absolute -right-0.5 -bottom-0.5 flex h-2.5 w-2.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-[var(--accent-green)] opacity-75" />
            <span className="relative inline-flex h-2.5 w-2.5 rounded-full bg-[var(--accent-green)]" />
          </span>
        }
        title={t('logCenter.title')}
        subtitle={t('logCenter.subtitle')}
        actions={
          <div className="flex items-center rounded-xl border border-[var(--border)] bg-[var(--bg-surface-solid)] p-1 shadow-2xs">
            {TABS.map((tab) => {
              const Icon = tab.icon
              const isActive = activeTab === tab.id
              return (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => setActiveTab(tab.id)}
                  className={`relative flex items-center gap-1.5 rounded-lg px-3.5 py-1.5 text-xs font-medium transition-colors ${
                    isActive
                      ? 'text-white'
                      : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                  }`}
                >
                  {isActive && (
                    <motion.div
                      layoutId="log-tab-active-pill"
                      className="absolute inset-0 rounded-lg bg-[var(--accent)] shadow-xs"
                      transition={
                        reducedMotion
                          ? { duration: 0 }
                          : { type: 'spring', stiffness: 500, damping: 35 }
                      }
                    />
                  )}
                  <Icon className="relative z-10 h-3.5 w-3.5" />
                  <span className="relative z-10">{t(tab.labelKey)}</span>
                </button>
              )
            })}
          </div>
        }
      />

      {/* 主工作区 */}
      <div className="relative min-h-0 flex-1 overflow-hidden">
        <AnimatePresence mode="wait">
          <motion.div
            key={activeTab}
            initial={reducedMotion ? false : { opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={reducedMotion ? undefined : { opacity: 0, y: -6 }}
            transition={{ duration: 0.18, ease: 'easeOut' }}
            className="flex h-full w-full flex-col overflow-hidden"
          >
            {activeTab === 'operations' ? <OperationLogsTab /> : <OperationalLogsTab />}
          </motion.div>
        </AnimatePresence>
      </div>
    </div>
  )
}
