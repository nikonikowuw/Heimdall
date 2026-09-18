export function isCosineThresholdKey(key: string): boolean {
  return key === 'similarity_threshold' || key === 'review_threshold'
}

/**
 * 将原始 cosine similarity 从 [-1, 1] 映射到 [0, 1]。
 */
export function normalizeCosineSimilarity(similarity: number): number {
  return (Math.min(1, Math.max(-1, similarity)) + 1) / 2
}

/**
 * 将展示层 [0, 1] 分值转换回运行时使用的原始 cosine similarity。
 */
export function denormalizeCosineSimilarity(normalized: number): number {
  return Math.min(1, Math.max(0, normalized)) * 2 - 1
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

/**
 * 根据原始余弦相似度在展示层归一化后的分值确定置信度色彩阶:
 * - high (>= 80%): 高置信度匹配 (翡翠绿)
 * - medium (60% ~ 80%): 疑似待复核 (暖橙色)
 * - low (< 60%): 低置信度/不匹配 (警示红)
 */
export function getCosineSimilarityLevel(similarity: number | null | undefined): SimilarityLevel {
  if (similarity == null || !Number.isFinite(similarity)) {
    return 'low'
  }
  const normalized = normalizeCosineSimilarity(similarity)
  if (normalized >= 0.8) {
    return 'high'
  }
  if (normalized >= 0.6) {
    return 'medium'
  }
  return 'low'
}
