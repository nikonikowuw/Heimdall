import type { AffinityIntent } from '@/types'

/**
 * 格式化 NPU 核心亲和徽标文本
 */
export function formatAffinityBadge(affinity?: AffinityIntent | null): string {
  if (!affinity || affinity.mode === 'auto') {
    const policy = (affinity && 'policy' in affinity && affinity.policy) || 'spread'
    return policy === 'pack' ? 'Auto·Pack' : 'Auto·Spread'
  }
  return `Core ${affinity.coreIndex}`
}
