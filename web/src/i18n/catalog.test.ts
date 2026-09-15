import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { SUPPORTED_LOCALES } from './index'

const I18N_DIR = fileURLToPath(new URL('.', import.meta.url))
const SRC_DIR = fileURLToPath(new URL('..', import.meta.url))

type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

/** 以 `en` 为基准推导全部 namespace */
const NAMESPACES = readdirSync(join(I18N_DIR, 'en'))
  .filter((name) => name.endsWith('.json'))
  .map((name) => name.slice(0, -'.json'.length))
  .sort()

function flatten(node: Json, prefix = ''): Map<string, string> {
  const out = new Map<string, string>()
  if (node !== null && typeof node === 'object' && !Array.isArray(node)) {
    for (const [key, value] of Object.entries(node)) {
      const path = prefix ? `${prefix}.${key}` : key
      if (value !== null && typeof value === 'object' && !Array.isArray(value)) {
        for (const [k, v] of flatten(value, path)) out.set(k, v)
      } else {
        out.set(path, String(value))
      }
    }
  }
  return out
}

function readCatalog(locale: string, namespace: string): Map<string, string> {
  const raw = readFileSync(join(I18N_DIR, locale, `${namespace}.json`), 'utf8')
  return flatten(JSON.parse(raw) as Json)
}

/** 递归收集来源目录下的 .ts / .tsx（跳过测试与类型声明） */
function collectSources(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    if (statSync(full).isDirectory()) {
      if (entry === 'i18n') continue
      out.push(...collectSources(full))
      continue
    }
    if (!/\.tsx?$/.test(entry)) continue
    if (/\.(test|spec)\.tsx?$/.test(entry) || entry.endsWith('.d.ts')) continue
    out.push(full)
  }
  return out
}

/** 匹配 t('some.key', ...) 形式的字面量键，跳过 t(variable) 动态调用 */
const T_CALL = /(?:^|[^\w.$])t\(\s*['"]([^'"]+)['"]/g

describe('i18n catalog parity', () => {
  it('exposes the expected namespaces', () => {
    expect(NAMESPACES.length).toBeGreaterThan(0)
  })

  for (const namespace of NAMESPACES) {
    it(`namespace "${namespace}" has identical keys across all locales`, () => {
      const reference = readCatalog(SUPPORTED_LOCALES[0], namespace)
      const referenceKeys = new Set(reference.keys())

      for (const locale of SUPPORTED_LOCALES.slice(1)) {
        const actualKeys = new Set(readCatalog(locale, namespace).keys())
        expect({
          locale,
          missing: [...referenceKeys].filter((key) => !actualKeys.has(key)).sort(),
          extra: [...actualKeys].filter((key) => !referenceKeys.has(key)).sort(),
        }).toEqual({ locale, missing: [], extra: [] })
      }
    })
  }
})

describe('i18n literal key resolution', () => {
  it('every t() literal in src resolves in every locale', () => {
    // 每个 locale 的全部 namespace 键求并集：源码里 t('common.cancel') 这类写法
    // 的归属 namespace 依赖运行时上下文，静态不可靠判定，因此只要在该语言的
    // 任一 namespace 中可解析即视为满足；真正要拦截的是「三语全缺、静默回退到
    // defaultValue（通常为中文）」这一类缺陷。
    const catalogs = SUPPORTED_LOCALES.map((locale) => ({
      locale,
      keys: new Set(NAMESPACES.flatMap((namespace) => [...readCatalog(locale, namespace).keys()])),
    }))

    const unresolved: Array<{ key: string; file: string; locales: string[] }> = []
    let scanned = 0

    for (const file of collectSources(SRC_DIR)) {
      for (const match of readFileSync(file, 'utf8').matchAll(T_CALL)) {
        scanned += 1
        const raw = match[1]
        const key = raw.includes(':') ? raw.slice(raw.indexOf(':') + 1) : raw
        const missingIn = catalogs.filter((c) => !c.keys.has(key)).map((c) => c.locale)
        if (missingIn.length > 0) {
          unresolved.push({ key, file: file.slice(SRC_DIR.length + 1), locales: missingIn })
        }
      }
    }

    // 非空洞断言：正则或目录遍历一旦失效，scanned 会塌缩，必须让测试显式失败
    expect(scanned).toBeGreaterThan(500)
    expect(unresolved).toEqual([])
  })
})
