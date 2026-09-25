import type { PersonnelImportProgress } from '@/types'

/**
 * 批量导入归档的受支持扩展名。
 *
 * 与后端 `api::algo::archive::detect_archive_format` 的候选集保持一致：
 * 前端只做上传前的前置提示，真正的格式识别仍以归档魔数为准。
 */
export const ACCEPTED_EXTENSIONS = ['.zip', '.tar.gz', '.tgz', '.tar'] as const

/** 校验归档文件名是否受支持（大小写不敏感） */
export function isSupportedArchiveName(filename: string): boolean {
  const lower = filename.toLowerCase()
  return ACCEPTED_EXTENSIONS.some((ext) => lower.endsWith(ext))
}

/** 终态判定：这些状态表示任务已结束，可展示报告 */
export function isTerminalImportStatus(status: PersonnelImportProgress['status']): boolean {
  return status === 'completed' || status === 'failed' || status === 'cancelled'
}
