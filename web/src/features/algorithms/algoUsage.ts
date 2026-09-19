import type { TaskSummaryDto } from '@/types'

/** 单个算法在某个通道任务上的占用记录 */
export interface AlgoUsageEntry {
  cameraId: string
  /** 任务名称（用户可读），用于卸载前置提示 */
  taskName: string
  /** 该实例的期望启用状态 */
  enabled: boolean
}

/**
 * 聚合每个算法被哪些通道任务占用。
 *
 * 后端在卸载被占用版本时才会返回「算法使用中」，界面只能在用户点击之后才知道；
 * 这里用任务清单里已有的算法实例提前算出占用关系，把失败前置为可解释的禁用态。
 * 同一算法在同一通道上的多实例只记一条，保证「N 路通道在用」的口径是通道数而不是实例数。
 */
export function buildAlgoUsage(tasks: TaskSummaryDto[]): Map<string, AlgoUsageEntry[]> {
  const pending = new Map<string, Map<string, AlgoUsageEntry>>()

  for (const task of tasks) {
    for (const instance of task.algorithmInstances ?? []) {
      const algorithmId = instance.algorithmId?.trim()
      if (!algorithmId) continue

      let byCamera = pending.get(algorithmId)
      if (!byCamera) {
        byCamera = new Map<string, AlgoUsageEntry>()
        pending.set(algorithmId, byCamera)
      }
      if (byCamera.has(task.cameraId)) continue

      byCamera.set(task.cameraId, {
        cameraId: task.cameraId,
        taskName: task.name,
        enabled: instance.enabled,
      })
    }
  }

  const usage = new Map<string, AlgoUsageEntry[]>()
  for (const [algorithmId, byCamera] of pending) {
    usage.set(
      algorithmId,
      [...byCamera.values()].sort((a, b) => a.cameraId.localeCompare(b.cameraId)),
    )
  }
  return usage
}

/** 取指定算法的占用通道列表，未占用时返回空数组 */
export function usageEntriesFor(
  usage: ReadonlyMap<string, AlgoUsageEntry[]>,
  algorithmId: string,
): AlgoUsageEntry[] {
  return usage.get(algorithmId) ?? []
}

/**
 * 会阻断卸载的占用项。
 *
 * 后端 `count_active_instances` 只统计 `enabled = true` 的实例，因此
 * 「在用」的口径必须是启用中的实例；已绑定但停用的通道只是上下文信息，不阻断操作。
 */
export function blockingUsageEntries(entries: AlgoUsageEntry[]): AlgoUsageEntry[] {
  return entries.filter((entry) => entry.enabled)
}
