import { useState, useEffect, useRef, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { systemApi } from '../../../lib/system-api'
import type { NetworkChangeOperation } from '../../../types/system'

interface UseNetworkTrialProps {
  onReload: () => Promise<void>
}

export interface UseNetworkTrialReturn {
  pendingOp: NetworkChangeOperation | null
  syncPendingOp: (op: NetworkChangeOperation | null) => void
  trialError: string | null
  clearTrialError: () => void
  confirmTrial: (id: string) => Promise<void>
  cancelTrial: (id: string) => Promise<void>
}

export function useNetworkTrial({ onReload }: UseNetworkTrialProps): UseNetworkTrialReturn {
  const { t } = useTranslation('system')
  const [pendingOp, setPendingOp] = useState<NetworkChangeOperation | null>(null)
  const [trialError, setTrialError] = useState<string | null>(null)
  const pollTimerRef = useRef<ReturnType<typeof setInterval> | null>(null)
  const onReloadRef = useRef(onReload)

  useEffect(() => {
    onReloadRef.current = onReload
  }, [onReload])

  const syncPendingOp = useCallback((op: NetworkChangeOperation | null) => {
    setPendingOp(op)
  }, [])

  // 处于试运行状态时自动轮询未决操作与看门狗心跳
  useEffect(() => {
    if (!pendingOp || pendingOp.status !== 'pending_confirm') {
      if (pollTimerRef.current) {
        clearInterval(pollTimerRef.current)
        pollTimerRef.current = null
      }
      return
    }

    pollTimerRef.current = setInterval(async () => {
      try {
        const currentOp = await systemApi.getPendingNetworkChange()
        setPendingOp(currentOp)
        if (!currentOp || currentOp.status !== 'pending_confirm') {
          await onReloadRef.current()
        }
      } catch {
        // 网络重连期异常静默忽略
      }
    }, 2500)

    return () => {
      if (pollTimerRef.current) {
        clearInterval(pollTimerRef.current)
        pollTimerRef.current = null
      }
    }
  }, [pendingOp])

  const confirmTrial = useCallback(
    async (id: string) => {
      try {
        setTrialError(null)
        await systemApi.confirmNetworkChange(id)
        setPendingOp(null)
        await onReloadRef.current()
      } catch (err) {
        setTrialError(
          err instanceof Error
            ? err.message
            : t('network.confirmFailed', { defaultValue: '确认配置失败' }),
        )
      }
    },
    [t],
  )

  const cancelTrial = useCallback(
    async (id: string) => {
      try {
        setTrialError(null)
        await systemApi.cancelNetworkChange(id)
        setPendingOp(null)
        await onReloadRef.current()
      } catch (err) {
        setTrialError(
          err instanceof Error
            ? err.message
            : t('network.cancelFailed', { defaultValue: '放弃配置失败' }),
        )
      }
    },
    [t],
  )

  const clearTrialError = useCallback(() => {
    setTrialError(null)
  }, [])

  return {
    pendingOp,
    syncPendingOp,
    trialError,
    clearTrialError,
    confirmTrial,
    cancelTrial,
  }
}
