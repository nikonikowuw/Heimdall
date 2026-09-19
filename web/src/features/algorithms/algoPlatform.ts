/**
 * 归一化平台代号 → 加速单元标签。
 *
 * 这些是各芯片厂商的产品代号（CoreML / RKNN / CANN），任何语言下都保持原样；
 * 未知代号原样回显，便于后端新增平台时现场仍能核对到底是哪个宿主。
 */
const HOST_ACCELERATOR_LABELS: Record<string, string> = {
  'macos-arm64': 'CoreML / ANE',
  'linux-rknn': 'RKNN NPU',
  'linux-ascend': 'CANN NPU',
  'linux-x64': 'CPU',
}

/** 描述宿主平台的加速单元；空代号返回空串由调用方决定占位符 */
export function describeHostAccelerator(normalizedPlatformId: string): string {
  const id = normalizedPlatformId.trim()
  if (!id) return ''
  return HOST_ACCELERATOR_LABELS[id] ?? id
}
