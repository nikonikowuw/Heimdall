export function isCosineThresholdKey(key: string): boolean {
  return key === 'similarity_threshold' || key === 'review_threshold'
}

/**
 * 工业级人脸识别三锚点分段线性标定（Piecewise Linear Calibration）：
 *
 * 锚点物理定义（基于 512 维单位超球面特征内积分布）：
 * - 负样本底线 (x0 = 0.20 -> y0 = 0%):
 *     陌生人/负样本对的余弦相似度集中在 [0.0, 0.25]，把 <= 0.20 截断标定为 0 分，
 *     彻底消除 (raw+1)/2 导致陌生人平白获得 50~60 分的直觉认知失真。
 * - 疑似复核门限 (x1 = 0.50 -> y1 = 60%):
 *     将 review_threshold (0.50) 严格锚定为业务及格线 60 分（疑似复核区起点）。
 * - 确认放行门限 (x2 = 0.60 -> y2 = 80%):
 *     将 similarity_threshold (0.60) 严格锚定为高置信放行线 80 分（高置信放行区起点）。
 * - 极高置信上限 (x3 = 1.00 -> y3 = 100%):
 *     同图/极高清晰正脸理论上限内积 1.00 映射为满分 100 分。
 *
 * 各分段斜率：
 * - x <= 0.20: y = 0.0 (0%)
 * - 0.20 < x <= 0.60: y = (x - 0.20) * 2.0 (斜率恒为 2.0，涵盖低分与疑似复核区)
 * - 0.60 < x <= 1.00: y = 0.80 + (x - 0.60) * 0.50 (斜率为 0.50，高置信平滑展开)
 * - x > 1.00: y = 1.0 (100%)
 */
export function normalizeCosineSimilarity(similarity: number): number {
  if (!Number.isFinite(similarity) || similarity <= 0.2) {
    return 0
  }
  if (similarity < 0.6) {
    return (similarity - 0.2) * 2.0
  }
  if (similarity >= 1.0) {
    return 1.0
  }
  return 0.8 + (similarity - 0.6) * 0.5
}

/**
 * 将展示层百分比归一化分值 [0, 1] 严格逆映射回算法底层的原始余弦相似度（[-1, 1]），
 * 供控制台任务参数配置、阈值滑块双向绑定反解：
 * - y <= 0.0: 反解为 0.20
 * - 0.0 < y < 0.80: x = 0.20 + y * 0.50
 * - 0.80 <= y <= 1.00: x = 0.60 + (y - 0.80) * 2.0
 * - y > 1.00: 反解为 1.00
 */
export function denormalizeCosineSimilarity(normalized: number): number {
  if (!Number.isFinite(normalized) || normalized <= 0) {
    return 0.2
  }
  if (normalized < 0.8) {
    return 0.2 + normalized * 0.5
  }
  if (normalized >= 1.0) {
    return 1.0
  }
  return 0.6 + (normalized - 0.8) * 2.0
}

/**
 * 格式化用户界面中的归一化 cosine 相似度百分比。
 */
export function formatCosineSimilarityPercent(
  similarity: number | null | undefined,
  fractionDigits = 1,
): string {
  if (similarity == null || !Number.isFinite(similarity)) {
    return '-'
  }

  return `${(normalizeCosineSimilarity(similarity) * 100).toFixed(fractionDigits)}%`
}

export type SimilarityLevel = 'high' | 'medium' | 'low'

const EPSILON = 1e-6

/**
 * 根据原始余弦相似度在展示层归一化后的分值确定置信度色彩阶:
 * - high (>= 80%): 高置信度匹配 (翡翠绿，对应 raw >= 0.60 确认放行)
 * - medium (60% ~ 80%): 疑似待复核 (暖橙色，对应 0.50 <= raw < 0.60 待核验)
 * - low (< 60%): 低置信度/不匹配 (警示红，对应 raw < 0.50 未命中)
 */
export function getCosineSimilarityLevel(similarity: number | null | undefined): SimilarityLevel {
  if (similarity == null || !Number.isFinite(similarity)) {
    return 'low'
  }
  const normalized = normalizeCosineSimilarity(similarity)
  if (normalized >= 0.8 - EPSILON) {
    return 'high'
  }
  if (normalized >= 0.6 - EPSILON) {
    return 'medium'
  }
  return 'low'
}
