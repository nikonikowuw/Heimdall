/**
 * 字面量联合的边界收窄工具。
 *
 * `<select>`、WS payload 与配置解析都会拿到宽泛的 `string`，收窄回字面量联合必须靠
 * 运行时校验 —— `as` 断言在这些场景是静默失效的入口：选项表与类型一旦漂移，
 * 断言仍会放行非法值，而校验只会拒绝。
 */

/**
 * 判断 `value` 是否属于 `options` 声明的取值集合。
 *
 * 全仓唯一的联合成员判定实现：oplog 的 WS 载荷归一化、以及各筛选下拉的 `<select>`
 * 取值都走这里，避免每个 feature 各写一份等价谓词。
 */
export function isMemberOf<T extends string>(options: readonly T[], value: string): value is T {
  return (options as readonly string[]).includes(value)
}

/** 下拉选项：`value` 即该下拉允许出现的取值集合 */
export interface SelectOptionLike<T extends string> {
  value: T
}

/**
 * 从下拉选项中收窄取值，可附加一个独立的「全部 / 不限」档。
 *
 * 与 [isMemberOf] 的分工：后者用于「值域已在别处声明」的校验（如 WS 载荷），
 * 本函数用于「选项数组本身就是唯一真源」的场景，直接返回匹配到的字面量。
 *
 * @param raw 原生控件回传的原始字符串
 * @param options 已声明的选项；不匹配时返回 `undefined`
 * @param allOption 可选的「全部」档，其 `value` 常为空串，因此单独比对
 */
export function narrowSelectValue<T extends string>(
  raw: string,
  options: ReadonlyArray<SelectOptionLike<T>>,
  allOption?: SelectOptionLike<T>,
): T | undefined {
  if (allOption && allOption.value === raw) return allOption.value
  return options.find((option) => option.value === raw)?.value
}
