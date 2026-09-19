import { useEffect, useMemo, useState } from 'react'
import { taskApi } from '@/lib/api'
import type { TaskSummaryDto } from '@/types'
import { buildAlgoUsage, type AlgoUsageEntry } from '../algoUsage'

export interface UseAlgoUsageResult {
  usage: ReadonlyMap<string, AlgoUsageEntry[]>
  isLoading: boolean
  hasError: boolean
}

/**
 * 算法占用关系（哪些通道任务正在使用某个算法）。
 *
 * 数据来自任务清单里已存在的算法实例，因此不需要新增接口；占用关系决定
 * 「卸载版本」能否点击，必须在用户尝试之前就可见，而不是等后端返回「算法使用中」。
 * 任务清单整体取回后在前端聚合，任务规模上限由接入通道数决定（边缘设备为个位数到百级）。
 */
export function useAlgoUsage(refreshVersion: number): UseAlgoUsageResult {
  const [tasks, setTasks] = useState<TaskSummaryDto[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [hasError, setHasError] = useState(false)

  useEffect(() => {
    let alive = true
    setIsLoading(true)
    setHasError(false)
    void taskApi
      .list()
      .then((list) => {
        if (!alive) return
        setTasks(list)
        setHasError(false)
      })
      .catch(() => {
        // 保留上次成功快照，但未知期间必须锁定卸载，不能把空映射误认为安全。
        if (alive) setHasError(true)
      })
      .finally(() => {
        if (alive) setIsLoading(false)
      })
    return () => {
      alive = false
    }
  }, [refreshVersion])

  const usage = useMemo(() => buildAlgoUsage(tasks), [tasks])
  return { usage, isLoading, hasError }
}
