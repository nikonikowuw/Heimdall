/** 算法 schema 中用于声明目标类别的常见字段名 */
const TARGET_CLASS_KEYS = ['target_classes', 'allowed_classes', 'classes'] as const

/**
 * 旧版本前端在「应用参数」时无条件写入的键（含 camelCase 变体）。
 *
 * 这些键与参数应该出现在哪里无关：参数的位置与控件形态一律由 schema 决定，
 * 不再由键名决定。此表仅用于清理历史脏值——凡是 schema 未声明的键都剥掉，
 * 避免把前端臆造、算法包并不读取的字段持续下发。
 */
export const LEGACY_INJECTED_PARAM_KEYS = [
  'confidence_threshold',
  'confidenceThreshold',
  'detection_confidence_threshold',
  'target_classes',
  'targetClasses',
] as const

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

/**
 * 剥离 schema 未声明的历史注入键。
 *
 * schema 声明过的键原样保留（例如 general_detection 确实声明了
 * `confidence_threshold` / `target_classes`），只删除没有包声明过的臆造字段。
 */
export function stripLegacyInjectedParams(
  params: Record<string, unknown>,
  properties: Record<string, unknown>,
): Record<string, unknown> {
  const stripped = { ...params }
  for (const key of LEGACY_INJECTED_PARAM_KEYS) {
    if (!(key in properties)) {
      delete stripped[key]
    }
  }
  return stripped
}

/**
 * 依据算法 configSchema 生成初始参数。
 *
 * 只写 schema 声明的键：array/enum 型优先取 `default`，无默认值时回退到全部候选项
 * （「未选择即全选」由算法包声明的候选集表达），其余类型取 default / minimum / 类型零值。
 * 严禁在此注入 schema 未声明的键名，否则会下发算法包并不读取的字段。
 */
export function buildSchemaDefaultParams(
  version?: AlgoConfigSource | null,
): Record<string, unknown> {
  const properties = extractConfigProperties(version)
  const params: Record<string, unknown> = {}

  for (const [key, prop] of Object.entries(properties)) {
    if (getEnumOptions(prop).length > 0) {
      params[key] = resolveEnumSelection(prop, undefined)
      continue
    }
    if (prop.default !== undefined) {
      params[key] = prop.default
      continue
    }
    if (prop.type === 'number' || prop.type === 'integer') {
      params[key] = prop.minimum ?? 0
      continue
    }
    if (prop.type === 'boolean') {
      params[key] = false
      continue
    }
    if (prop.type === 'string') {
      params[key] = (isStringArray(prop.enum) && prop.enum[0]) || ''
    }
  }

  return params
}

/** 读取 array 型参数的候选项（`items.enum`）；非字符串枚举返回空数组 */
export function getEnumOptions(prop: Record<string, unknown>): string[] {
  const items = isRecord(prop.items) ? prop.items : undefined
  const options = items?.enum
  return isStringArray(options) && options.length > 0 ? [...options] : []
}

/**
 * 解析 array 型参数的选中项。
 *
 * 优先采用运行时值（空数组是用户「清空」的合法结果，必须尊重），
 * 其次 schema 默认值，最后回退到全部候选项。
 */
export function resolveEnumSelection(prop: Record<string, unknown>, raw: unknown): string[] {
  if (isStringArray(raw)) return [...raw]
  if (isStringArray(prop.default)) return [...prop.default]
  return getEnumOptions(prop)
}
