import React from 'react'
import { UserCheck } from 'lucide-react'
import type { RecognitionRecord } from '../../../types'
import { RecognitionCardItem } from './RecognitionCardItem'

export interface RecognitionContentProps {
  recognitions: RecognitionRecord[]
  cameraNameMap?: Record<string, string>
  onOpenReview: (rec: RecognitionRecord) => void
  onQuickReview: (rec: RecognitionRecord, status: 'confirmed' | 'rejected') => void
  t: (key: string) => string
}

export function RecognitionContent({
  recognitions,
  cameraNameMap,
  onOpenReview,
  onQuickReview,
  t,
}: RecognitionContentProps): React.ReactElement {
  if (recognitions.length === 0) {
    return (
      <div className="py-24 text-center text-[var(--text-muted)]">
        <UserCheck className="mx-auto mb-2 h-8 w-8 opacity-40" />
        <p className="font-medium text-[var(--text-secondary)]">{t('empty.recognitions')}</p>
        <p className="text-xs opacity-75">{t('empty.recognitionsDesc')}</p>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-1 gap-4 md:grid-cols-2 lg:grid-cols-3">
      {recognitions.map((recognition) => (
        <RecognitionCardItem
          key={recognition.recognitionId || recognition.id}
          recognition={recognition}
          cameraName={cameraNameMap?.[recognition.cameraId]}
          onOpenReview={onOpenReview}
          onQuickReview={onQuickReview}
          t={t}
        />
      ))}
    </div>
  )
}
