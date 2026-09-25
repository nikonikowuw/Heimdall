import React from 'react'
import { AnimatePresence } from 'motion/react'
import { useTranslation } from 'react-i18next'
import { ToastItemView } from '@/components/ui/Toast'
import type { ToastItem, ToastType } from '@/stores/toast'

export type PersonnelNoticeType = ToastType

export interface PersonnelNotice {
  id: number
  title: string
  message: string
  type: PersonnelNoticeType
  /** 后台任务收尾时的「查看报告」行动项；两类任务分别指向各自的报告视图 */
  action?: 'report' | 'importReport'
}

export interface PersonnelToastProps {
  notice: PersonnelNotice | null
  onDismiss: () => void
  onOpenReport: () => void
  onOpenImportReport?: () => void
}

/**
 * 人员管理即时反馈卡片（适配器模式，复用统一 ToastItemView 视觉与交互规范）。
 */
export function PersonnelToast({
  notice,
  onDismiss,
  onOpenReport,
  onOpenImportReport,
}: PersonnelToastProps): React.ReactElement | null {
  const { t } = useTranslation(['personnel', 'common'])

  if (!notice) return null

  let action: ToastItem['action']
  if (notice.action === 'report') {
    action = {
      label: t('reextract.viewReport', { defaultValue: '查看提取报告' }),
      onClick: () => {
        onDismiss()
        onOpenReport()
      },
      primary: true,
    }
  } else if (notice.action === 'importReport') {
    action = {
      label: t('import.viewReport', { defaultValue: '查看导入报告' }),
      onClick: () => {
        onDismiss()
        onOpenImportReport?.()
      },
      primary: true,
    }
  }

  const item: ToastItem = {
    id: String(notice.id),
    type: notice.type,
    title: notice.title,
    message: notice.message,
    category: t('common:nav.personnel', { defaultValue: '人员底库' }),
    duration: 0,
    action,
    createdAt: notice.id,
  }

  return (
    <div className="fixed top-4 right-4 z-[80] w-[min(92vw,26rem)] sm:top-5 sm:right-5">
      <AnimatePresence mode="wait">
        <ToastItemView key={item.id} item={item} onDismiss={onDismiss} />
      </AnimatePresence>
    </div>
  )
}
