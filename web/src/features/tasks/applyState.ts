import type { InstanceApplyState } from '@/types'

/**
 * 参与配置收敛判定的实例最小视图。
 *
 * `TaskAlgorithmInstanceDto`（任务配置）与 `TaskAlgorithmInstanceSummaryDto`（任务列表）
 * 都结构兼容，两处可直接传入，避免为展示再定义一层 DTO。
 */
export interface InstanceApplyView {
  algorithmId?: string
  enabled?: boolean
  applyState?: InstanceApplyState
  statusMessage?: string
}

/** 配置收敛态势：已生效 / 排队中 / 未生效 */
export type ApplyTone = 'ok' | 'pending' | 'failed'

export interface UnappliedInstance {
  algorithmId: string
  tone: 'pending' | 'failed'
  /** 服务端给出的可读原因，缺失时为空串 */
  reason: string
}

export interface InstanceApplySummary {
  tone: ApplyTone
  /** 未生效的启用实例；`failed` 排在 `pending` 之前，同级按算法标识稳定排序 */
  unapplied: UnappliedInstance[]
}

const TONE_RANK: Record<ApplyTone, number> = { ok: 0, pending: 1, failed: 2 }

/**
 * 归纳任务内所有启用实例的配置收敛状态。
 *
 * 判定规则与服务端保持一致的语义边界：
 * - 停用实例不参与判定：它的期望配置随下次启动生效，不代表当前运行异常；
 * - 缺失 `applyState`（旧版接口或未加载）视为已生效，避免把未知状态渲染成故障；
 * - 只要存在未生效实例就返回最严重的态势，供上层决定「静默 / 排队中 / 未生效」的展示。
 */
export function summarizeInstanceApply(
  instances: readonly InstanceApplyView[],
): InstanceApplySummary {
  const unapplied: UnappliedInstance[] = []
  let tone: ApplyTone = 'ok'

  for (const instance of instances) {
    if (instance.enabled === false) continue
    const state = instance.applyState
    if (state !== 'pending' && state !== 'failed') continue

    unapplied.push({
      algorithmId: instance.algorithmId ?? '',
      tone: state,
      reason: (instance.statusMessage ?? '').trim(),
    })
    if (TONE_RANK[state] > TONE_RANK[tone]) {
      tone = state
    }
  }

  unapplied.sort(
    (a, b) => TONE_RANK[b.tone] - TONE_RANK[a.tone] || a.algorithmId.localeCompare(b.algorithmId),
  )

  return { tone, unapplied }
}

/**
 * 生成悬停提示的逐行文案：每行一个算法与其未生效原因。
 *
 * 只做「算法标识 + 服务端原因」的数据拼装，不在前端拼接面向用户的句子，
 * 原因缺失时退化为算法标识本身。
 */
export function unappliedNoticeLines(summary: InstanceApplySummary): string[] {
  return summary.unapplied.map((item) =>
    item.reason ? `${item.algorithmId}: ${item.reason}` : item.algorithmId,
  )
}
