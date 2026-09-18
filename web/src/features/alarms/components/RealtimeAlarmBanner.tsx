import React from 'react'
import type { TFunction } from 'i18next'
import { ArrowUp, BellRing, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { motionTokens } from '@/lib/motionTokens'

export interface RealtimeAlarmBannerProps {
  count: number
  onViewNew: () => void
  onDismiss: () => void
  t: TFunction
}

export function RealtimeAlarmBanner({
  count,
  onViewNew,
  onDismiss,
  t,
}: RealtimeAlarmBannerProps): React.ReactElement {
  const shouldReduce = useReducedMotion()

  return (
    <AnimatePresence>
      {count > 0 && (
        <motion.div
          initial={{ opacity: 0, y: shouldReduce ? 0 : -8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: shouldReduce ? 0 : -8 }}
          transition={{
            duration: shouldReduce ? 0.1 : motionTokens.duration.fast,
            ease: motionTokens.easing.smooth,
          }}
          className="flex items-center justify-between rounded-xl border border-rose-500/40 bg-rose-500/15 px-4 py-2 text-xs text-rose-400 shadow-md backdrop-blur-md"
        >
          <div className="flex items-center gap-2">
            <BellRing className="h-4 w-4 animate-bounce text-rose-500" />
            <span className="font-semibold">{t('realtime.newAlarms', { count })}</span>
          </div>

          <div className="flex items-center gap-2">
            <motion.button
              type="button"
              whileTap={{ scale: 0.96 }}
              onClick={onViewNew}
              className="flex items-center gap-1 rounded-lg bg-rose-500 px-3 py-1 font-semibold text-white shadow-xs transition-opacity hover:opacity-90"
            >
              <span>{t('realtime.viewNew')}</span>
              <ArrowUp className="h-3.5 w-3.5" />
            </motion.button>

            <button
              type="button"
              onClick={onDismiss}
              className="rounded-lg p-1 text-rose-400 transition-colors hover:bg-rose-500/20 hover:text-white"
              title={t('realtime.dismiss')}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
