/**
 * 阈值类参数键：其取值与底库检索输出同处「标定置信分域」[0, 1]
 * （0.50 = 512 维正交基准，0.78 = 确认放行锚点）。
 */
export function isCosineThresholdKey(key: string): boolean {
  return key === 'similarity_threshold' || key === 'review_threshold'
}

/**
 * 标定置信分 [0, 1] → 展示百分比。
 *
 * 阈值参数与底库检索结果（`candidates[].similarity`）共用同一度量域，
 * 控制台只允许做 ×100 线性缩放：宿主 `capture_service` 会把
 * `params_json.similarity_threshold` 当作**标定置信分**直接消费，
 * 若在此套用原始余弦标定曲线或其逆运算，界面上设置的 75% 会在宿主侧
 * 被解读为 45.6%，确认门槛被静默下调近 30 个点。
 */
export function scoreToPercent(score: number): number {
  return score * 100
}

/**
 * 展示百分比 → 标定置信分 [0, 1]，[`scoreToPercent`] 的精确逆运算。
 */
export function percentToScore(percent: number): number {
  return percent / 100
}

/**
 * 格式化用户界面中的标定相似度/置信度百分比。
 * 算法包与后端通过 C ABI 底库直接输出 [0.0, 1.0] 的标定置信分值，前端直接格式化为百分比展示。
 */
export function formatCosineSimilarityPercent(
  similarity: number | null | undefined,
  fractionDigits = 1,
): string {
  if (similarity == null || !Number.isFinite(similarity)) {
    return '-'
  }

  const score = Math.max(0, Math.min(1, similarity))
  return `${(score * 100).toFixed(fractionDigits)}%`
}

export type SimilarityLevel = 'high' | 'medium' | 'low'

const EPSILON = 1e-6

/**
 * 根据标定置信度分值确定展示层色彩阶:
 * - high (>= 78%): 高置信度匹配 (翡翠绿，对应确认放行)
 * - medium (60% ~ 78%): 疑似待复核 (暖橙色，对应待核验)
 * - low (< 60%): 低置信度/不匹配 (警示红/中性灰，涵盖 50~55 分负样本底噪)
 */
export function getCosineSimilarityLevel(similarity: number | null | undefined): SimilarityLevel {
  if (similarity == null || !Number.isFinite(similarity)) {
    return 'low'
  }
  const score = Math.max(0, Math.min(1, similarity))
  if (score >= 0.78 - EPSILON) {
    return 'high'
  }
  if (score >= 0.6 - EPSILON) {
    return 'medium'
  }
  return 'low'
}
