/** 算法 schema 中用于声明目标类别的常见字段名 */
const TARGET_CLASS_KEYS = ['target_classes', 'allowed_classes', 'classes'] as const

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === 'string')
}

/**
 * 算法配置来源。字段声明为 `unknown`：入参来自后端 JSON，属不可信边界，
 * 由本模块负责结构校验而非依赖调用方断言。
 */
export interface AlgoConfigSource {
  configSchema?: unknown
  manifestRaw?: unknown
}

/** 提取算法 configSchema 的 properties 表；缺失或结构异常时返回空表 */
export function extractConfigProperties(
  version?: AlgoConfigSource | null,
): Record<string, Record<string, unknown>> {
  if (!version) return {}
  const schema = isRecord(version.configSchema) ? version.configSchema : {}
  const properties = schema.properties
  if (!isRecord(properties)) return {}

  const result: Record<string, Record<string, unknown>> = {}
  for (const [key, value] of Object.entries(properties)) {
    if (isRecord(value)) {
      result[key] = value
    }
  }
  return result
}

/**
 * 解析算法可识别的目标类别。
 * 优先读取 configSchema 中 target_classes / allowed_classes / classes 的枚举，
 * 其次回退到 manifestRaw.classes；两者都缺失时返回空数组（表示不做类别过滤）。
 */
export function extractTargetClasses(version?: AlgoConfigSource | null): string[] {
  if (!version) return []

  const properties = extractConfigProperties(version)
  for (const key of TARGET_CLASS_KEYS) {
    const prop = properties[key]
    if (!prop) continue
    const items = isRecord(prop.items) ? prop.items : undefined
    const enumClasses = items?.enum
    if (isStringArray(enumClasses) && enumClasses.length > 0) {
      return [...enumClasses]
    }
  }

  const rawClasses = isRecord(version.manifestRaw) ? version.manifestRaw.classes : undefined
  if (isStringArray(rawClasses) && rawClasses.length > 0) {
    return [...rawClasses]
  }

  return []
}
