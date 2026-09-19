/** 后端沙箱支持的归档扩展名（与上传接口的 accept 白名单同源） */
export const ALGO_PACKAGE_EXTENSIONS = ['.tar.gz', '.tgz', '.tar', '.zip'] as const

/** 推荐体积上限；超过后由服务端按配置的最大限制硬拦截，这里只做本地即时反馈 */
export const ALGO_PACKAGE_RECOMMENDED_MAX_BYTES = 500 * 1024 * 1024

/** 本地预检失败原因；映射到 i18n 文案由调用方负责 */
export type AlgoPackageRejection = 'unsupportedFormat' | 'emptyFile' | 'tooLarge'

/**
 * 归档包本地预检。
 *
 * 拖拽投递会绕过 `accept` 属性，因此格式白名单必须在代码里再做一次；
 * 体积与格式都以本地预检给出即时反馈，能否真正加载仍由服务端六步沙箱判定。
 */
export function validateAlgoPackage(
  fileName: string,
  sizeBytes: number,
): AlgoPackageRejection | null {
  const name = fileName.trim().toLowerCase()
  if (!name || !ALGO_PACKAGE_EXTENSIONS.some((ext) => name.endsWith(ext))) {
    return 'unsupportedFormat'
  }
  if (sizeBytes <= 0) return 'emptyFile'
  if (sizeBytes > ALGO_PACKAGE_RECOMMENDED_MAX_BYTES) return 'tooLarge'
  return null
}
