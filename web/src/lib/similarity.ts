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
