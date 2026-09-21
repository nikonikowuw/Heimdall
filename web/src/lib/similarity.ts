export function isCosineThresholdKey(key: string): boolean {
  return key === 'similarity_threshold' || key === 'review_threshold'
}

/**
 * 旷视同款人脸识别置信度分段线性标定（Megvii-style Calibration）：
 *
 * 锚点物理定义（基于 512 维超球面高维统计与旷视 Face++ 工业界实践）：
 * - 正交随机底线 (x0 = 0.00 -> y0 = 50.0%):
 *     在 512 维空间中，两张无关陌生人人脸特征呈近似正交 (余弦期望为 0)，
 *     映射为 50.0% 先验抛硬币状态 (无偏基准)。
 * - 底库负样本上限 (x1 = 0.10 -> y1 = 55.0%):
 *     底库非目标候选人余弦底噪集中在 [0.02, 0.10]，斜率 0.50，
 *     精准覆盖实测候选人分布：白俊聪 (0.026 -> 51.3%, 0.050 -> 52.5%)、许丽娜 (0.052 -> 52.6%)、林青 (0.102 -> 55.1%)。
 * - 疑似复核门限 (x2 = 0.40 -> y2 = 68.0%):
 *     将 review_threshold (0.40) 锚定为 68.0% (对标旷视 1e-4 门限 69.1)。
 * - 确认放行门限 (x3 = 0.48 -> y3 = 78.0%):
 *     将 similarity_threshold (0.48) 锚定为 78.0% (对标旷视 1e-5 金融级门限 74~75)。
 * - 时域融合近景门限 (x4 = 0.58 -> y4 = 88.4%):
 *     近景抓拍与时域球面融合区间 (0.50~0.58) 映射为 80.4% ~ 88.4% 高分。
 * - 极高置信上限 (x5 = 1.00 -> y5 = 100.0%):
 *     同图/近乎相同照片的理论上限 1.00 映射为 100.0%。
 *
 * 分段斜率与解析公式：
 * - x <= -1.0: y = 0.0
 * - -1.0 < x < 0.10: y = 0.50 + x * 0.50 (斜率 0.50)
 * - 0.10 <= x < 0.40: y = 0.55 + (x - 0.10) * (13 / 30) (斜率 0.433)
 * - 0.40 <= x < 0.48: y = 0.68 + (x - 0.40) * 1.25 (斜率 1.25, 门禁陡增跃升)
 * - 0.48 <= x < 0.58: y = 0.78 + (x - 0.48) * 1.04 (斜率 1.04, 确认同人区)
 * - 0.58 <= x <= 1.00: y = 0.884 + (x - 0.58) * (29 / 105) (斜率 0.276, 平缓收敛)
 * - x > 1.00: y = 1.0
 */
export function normalizeCosineSimilarity(similarity: number): number {
  if (!Number.isFinite(similarity) || similarity <= -1.0) {
    return 0
  }
  if (similarity < 0.1) {
    return 0.5 + similarity * 0.5
  }
  if (similarity < 0.4) {
    return 0.55 + (similarity - 0.1) * (13 / 30)
  }
  if (similarity < 0.48) {
    return 0.68 + (similarity - 0.4) * 1.25
  }
  if (similarity < 0.58) {
    return 0.78 + (similarity - 0.48) * 1.04
  }
  if (similarity >= 1.0) {
    return 1.0
  }
  return 0.884 + (similarity - 0.58) * (29 / 105)
}

/**
 * 将展示层百分比归一化分值 [0, 1] 严格逆映射回算法底层的原始余弦相似度（[-1, 1]），
 * 供控制台任务参数配置、阈值滑块双向绑定反解：
 * - y <= 0.0: 反解为 -1.0
 * - 0.0 < y < 0.55: x = (y - 0.50) / 0.50
 * - 0.55 <= y < 0.68: x = 0.10 + (y - 0.55) * (30 / 13)
 * - 0.68 <= y < 0.78: x = 0.40 + (y - 0.68) / 1.25
 * - 0.78 <= y < 0.884: x = 0.48 + (y - 0.78) / 1.04
 * - 0.884 <= y <= 1.00: x = 0.58 + (y - 0.884) * (105 / 29)
 * - y > 1.00: 反解为 1.00
 */
export function denormalizeCosineSimilarity(normalized: number): number {
  if (!Number.isFinite(normalized) || normalized <= 0) {
    return -1.0
  }
  if (normalized < 0.55) {
    return (normalized - 0.5) / 0.5
  }
  if (normalized < 0.68) {
    return 0.1 + (normalized - 0.55) * (30 / 13)
  }
  if (normalized < 0.78) {
    return 0.4 + (normalized - 0.68) / 1.25
  }
  if (normalized < 0.884) {
    return 0.48 + (normalized - 0.78) / 1.04
  }
  if (normalized >= 1.0) {
    return 1.0
  }
  return 0.58 + (normalized - 0.884) * (105 / 29)
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
