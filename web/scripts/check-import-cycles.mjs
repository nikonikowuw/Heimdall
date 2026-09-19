#!/usr/bin/env node
/**
 * 模块图循环依赖检查。
 *
 * 为什么需要单独一个检查：类型层的环（`import type`）会被 TypeScript 在编译时擦除，
 * 运行时完全无影响，因此构建、lint、测试都不会报错。但它仍然掩盖了真实的层级倒置，
 * 并且会误导后续的工具链与读者。ESLint 的 `no-restricted-imports` 只能约束路径形状，
 * 抓不到环本身，所以用这个脚本补上。
 *
 * 判据与 `docs/nuwa/frontend/directory-structure.md` 的「层间依赖须单向 … 整个模块图
 * 不得存在循环依赖」一致；发现环时逐条打印并以退出码 1 结束。
 *
 * 用法：
 *   node scripts/check-import-cycles.mjs
 *   pnpm check:cycles
 *
 * 解析范围：`src/` 下全部 `.ts`/`.tsx`（含测试），只跟随相对路径与 `@/` 别名，
 * 不进入 node_modules。路径别名从 tsconfig 读取，不另行硬编码，避免与
 * `vite.config.ts` 的 alias 各写一份而漂移。
 */

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const WEB_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const SRC_ROOT = path.join(WEB_ROOT, 'src')
const TSCONFIG = path.join(WEB_ROOT, 'tsconfig.app.json')

/** 依次尝试的补全后缀，顺序即解析优先级 */
const RESOLVE_SUFFIXES = ['', '.ts', '.tsx', '/index.ts', '/index.tsx']
/** 静态 `from '...'`、动态 `import('...')`、类型位置 `import('...').T` 三类写法 */
const SPECIFIER_PATTERN = /(?:\bfrom\s*|\bimport\s*\(\s*)['"]([^'"]+)['"]/g

/**
 * 读取 tsconfig 中的 `paths` 并展开成「前缀 → 目标目录」映射。
 * tsconfig 允许注释，先剥掉再交给 JSON.parse。
 */
function loadAliases() {
  const raw = fs
    .readFileSync(TSCONFIG, 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:"'])\/\/[^\n]*/g, '$1')

  let parsed
  try {
    parsed = JSON.parse(raw)
  } catch (error) {
    console.error(`无法解析 ${path.relative(WEB_ROOT, TSCONFIG)}：${error.message}`)
    process.exit(1)
  }

  const baseUrl = path.resolve(WEB_ROOT, parsed.compilerOptions?.baseUrl ?? '.')
  const entries = Object.entries(parsed.compilerOptions?.paths ?? {}).map(([from, targets]) => [
    from.replace(/\/\*$/, '/'),
    path.resolve(baseUrl, targets[0].replace(/\/\*$/, '')),
  ])

  if (entries.length === 0) {
    console.error(`${path.relative(WEB_ROOT, TSCONFIG)} 未定义 compilerOptions.paths，无法解析别名`)
    process.exit(1)
  }
  return entries
}

/** 收集 src 下的全部 TypeScript 源文件 */
function collectModules(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name)
    if (entry.isDirectory()) return collectModules(full)
    return /\.tsx?$/.test(entry.name) ? [full] : []
  })
}

/** 把模块说明符解析为真实文件路径；解析不到（如 ./foo.css、外部包）返回 null */
function resolveSpecifier(fromFile, specifier, aliases) {
  let base = null
  for (const [prefix, target] of aliases) {
    if (specifier.startsWith(prefix)) {
      base = path.join(target, specifier.slice(prefix.length))
      break
    }
  }
  if (base === null) {
    if (!specifier.startsWith('.')) return null // 外部依赖
    base = path.resolve(path.dirname(fromFile), specifier)
  }

  for (const suffix of RESOLVE_SUFFIXES) {
    const candidate = base + suffix
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) return candidate
  }
  return null
}

/** 用深度优先搜索找出全部环，按环内模块集合去重 */
function findCycles(graph) {
  const cycles = new Set()
  const settled = new Set()

  const visit = (node, stack, onStack) => {
    if (onStack.has(node)) {
      const start = stack.indexOf(node)
      const cycle = stack.slice(start)
      cycles.add(JSON.stringify(cycle))
      return
    }
    if (settled.has(node)) return

    settled.add(node)
    onStack.add(node)
    stack.push(node)
    for (const next of graph.get(node) ?? []) visit(next, stack, onStack)
    stack.pop()
    onStack.delete(node)
  }

  for (const node of graph.keys()) visit(node, [], new Set())
  return [...cycles].map((entry) => JSON.parse(entry))
}

const aliases = loadAliases()
const modules = collectModules(SRC_ROOT)
const graph = new Map()
const unresolvedAliases = new Set()

let edgeCount = 0
for (const file of modules) {
  const source = fs.readFileSync(file, 'utf8')
  const dependencies = new Set()

  for (const match of source.matchAll(SPECIFIER_PATTERN)) {
    const specifier = match[1]
    const resolved = resolveSpecifier(file, specifier, aliases)
    if (resolved === null) {
      // 别名解析不出来通常意味着 paths 改了而脚本没跟上，必须显式暴露而非静默漏检
      if (aliases.some(([prefix]) => specifier.startsWith(prefix))) {
        unresolvedAliases.add(`${path.relative(WEB_ROOT, file)} → ${specifier}`)
      }
      continue
    }
    if (resolved !== file) dependencies.add(resolved)
  }

  graph.set(file, [...dependencies])
  edgeCount += dependencies.size
}

if (unresolvedAliases.size > 0) {
  console.error(`以下 ${unresolvedAliases.size} 个别名导入未能解析，检查可能不完整：`)
  for (const entry of unresolvedAliases) console.error(`  ${entry}`)
  process.exit(1)
}

const cycles = findCycles(graph)
const relative = (file) =>
  path.relative(SRC_ROOT, file).startsWith('..')
    ? path.relative(WEB_ROOT, file)
    : path.relative(SRC_ROOT, file)

if (cycles.length === 0) {
  console.log(`模块图无循环依赖（${modules.length} 个模块，${edgeCount} 条内部依赖）`)
  process.exit(0)
}

console.error(
  `发现 ${cycles.length} 处循环依赖（共 ${modules.length} 个模块，${edgeCount} 条内部依赖）：`,
)
for (const cycle of cycles) {
  console.error('')
  cycle.forEach((file, index) => console.error(`  ${index + 1}. ${relative(file)}`))
  console.error(`  ↳ 回到 ${relative(cycle[0])}`)
}
console.error('\n层级方向见 docs/nuwa/frontend/directory-structure.md 的「模块边界」。')
process.exit(1)
