import { useCallback, useEffect, useRef, useState } from 'react'
import { ApiError, oplogApi } from '../../../lib/api'
import type { OperationLog } from '../../../types'

const PAGE_SIZE = 50

export interface UseOplogsResult {
  logs: OperationLog[]
  isLoading: boolean
  isLoadingMore: boolean
  hasMore: boolean
  error: string | null
  refresh: () => void
  loadMore: () => void
}

export function getErrorMessage(error: unknown): string {
  if (error instanceof ApiError || error instanceof Error) {
    return error.message || 'Unknown error'
  }
  if (typeof error === 'string' && error.trim().length > 0) {
    return error
  }
  if (
    error &&
    typeof error === 'object' &&
    'message' in error &&
    typeof error.message === 'string' &&
    error.message.trim().length > 0
  ) {
    return error.message
  }
  return 'Unknown error'
}

export function mergeLogs(currentLogs: OperationLog[], nextLogs: OperationLog[]): OperationLog[] {
  const existingIds = new Set(currentLogs.map((log) => log.id))
  const newItems = nextLogs.filter((log) => !existingIds.has(log.id))
  return [...currentLogs, ...newItems]
}

export function useOplogs(moduleFilter: string): UseOplogsResult {
  const [logs, setLogs] = useState<OperationLog[]>([])
  const [offset, setOffset] = useState(0)
  const [isLoading, setIsLoading] = useState(true)
  const [isLoadingMore, setIsLoadingMore] = useState(false)
  const [hasMore, setHasMore] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [refreshVersion, setRefreshVersion] = useState(0)
  const generationRef = useRef(0)
  const loadMoreControllerRef = useRef<AbortController | null>(null)

  const executeFetch = useCallback(
    (targetOffset: number, isAppend: boolean, controller: AbortController, generation: number) => {
      if (isAppend) {
        setIsLoadingMore(true)
      } else {
        setIsLoading(true)
        setError(null)
      }

      void oplogApi
        .list(
          {
            module: moduleFilter || undefined,
            limit: PAGE_SIZE,
            offset: targetOffset,
          },
          controller.signal,
        )
        .then((nextLogs) => {
          if (generationRef.current !== generation) return
          setLogs((current) => (isAppend ? mergeLogs(current, nextLogs) : nextLogs))
          setOffset(targetOffset + nextLogs.length)
          setHasMore(nextLogs.length === PAGE_SIZE)
        })
        .catch((requestError: unknown) => {
          if (controller.signal.aborted || generationRef.current !== generation) return
          setError(getErrorMessage(requestError))
        })
        .finally(() => {
          if (generationRef.current === generation) {
            if (isAppend) {
              if (loadMoreControllerRef.current === controller) {
                loadMoreControllerRef.current = null
                setIsLoadingMore(false)
              }
            } else {
              setIsLoading(false)
            }
          }
        })
    },
    [moduleFilter],
  )

  useEffect(() => {
    const generation = generationRef.current + 1
    generationRef.current = generation
    const controller = new AbortController()
    loadMoreControllerRef.current?.abort()
    loadMoreControllerRef.current = null

    setLogs([])
    setOffset(0)
    setHasMore(false)

    executeFetch(0, false, controller, generation)

    return () => {
      controller.abort()
      loadMoreControllerRef.current?.abort()
      loadMoreControllerRef.current = null
      if (generationRef.current === generation) {
        generationRef.current += 1
      }
    }
  }, [executeFetch, refreshVersion])

  const refresh = useCallback(() => {
    setRefreshVersion((version) => version + 1)
  }, [])

  const loadMore = useCallback(() => {
    if (isLoading || isLoadingMore || !hasMore) return

    const controller = new AbortController()
    const generation = generationRef.current
    loadMoreControllerRef.current?.abort()
    loadMoreControllerRef.current = controller

    executeFetch(offset, true, controller, generation)
  }, [executeFetch, hasMore, isLoading, isLoadingMore, offset])

  return {
    logs,
    isLoading,
    isLoadingMore,
    hasMore,
    error,
    refresh,
    loadMore,
  }
}
