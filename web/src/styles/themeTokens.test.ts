import { describe, expect, it } from 'vitest'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative } from 'node:path'

const SRC_DIR = join(process.cwd(), 'src')
const CSS = readFileSync(join(SRC_DIR, 'styles/globals.css'), 'utf8')

/** 全部非测试源码，供跨文件契约检查使用 */
const SOURCES = (function collect(dir: string, acc: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry.startsWith('.')) continue
    const path = join(dir, entry)
    if (statSync(path).isDirectory()) collect(path, acc)
    else if (/\.tsx?$/.test(entry) && !/\.test\./.test(entry)) acc.push(path)
  }
  return acc
})(SRC_DIR)

/** 截取两个锚点之间的 CSS 片段；锚点缺失时返回空串，让断言直接失败而非静默通过 */
function sliceBetween(startAnchor: string, endAnchor: string): string {
  const start = CSS.indexOf(startAnchor)
  if (start === -1) return ''
  const end = CSS.indexOf(endAnchor, start)
  return CSS.slice(start, end === -1 ? undefined : end)
}

/** 从一段 CSS 中取出指定变量的值 */
function tokenValue(block: string, name: string): string | undefined {
  return block.match(new RegExp(`--${name}:\\s*([^;]+);`))?.[1].trim()
}

/** 逐行扫描全部源码，返回回调判定为违规的位置（`文件:行号`） */
function findOffences(test: (line: string) => boolean): string[] {
  const found: string[] = []
  for (const file of SOURCES) {
    const rel = relative(SRC_DIR, file)
    readFileSync(file, 'utf8')
      .split('\n')
      .forEach((line, index) => {
        if (test(line)) found.push(`${rel}:${index + 1}`)
      })
  }
  return found
}

const ROOT_VARS = sliceBetween(':root {', '.dark {')
const DARK_VARS = sliceBetween('.dark {', '@layer components')
const THEME_VARS = sliceBetween('@theme {', ':root {')
const DARK_SURFACE_VARS = sliceBetween('.on-dark-surface,', '.modal-backdrop--immersive {')

const STATUS_SLOTS = ['success', 'danger', 'warning', 'info', 'neutral']

/** 各状态槽在 :root 与 .dark 都必须成组定义，便于逐槽断言 */
describe('主题 token 契约', () => {
  // 这四个 token 的用途是「实底 + 白字」。暗色主题下的主题感知值（如 #34d399）
  // 对白字仅 1.92:1，不可读，因此必须存在独立且跨主题固定的 -solid 档。
  it.each(['success', 'danger', 'warning', 'info'])(
    'defines a cross-theme --status-%s-solid for white-foreground fills',
    (name) => {
      expect(
        tokenValue(ROOT_VARS, `status-${name}-solid`),
        `--status-${name}-solid 必须在 :root 定义`,
      ).toBeTruthy()
      // 暗色主题不得覆写 solid 档，否则实底会跟着变浅、白字失去对比
      expect(
        tokenValue(DARK_VARS, `status-${name}-solid`),
        `--status-${name}-solid 不应在 .dark 覆写`,
      ).toBeUndefined()
    },
  )

  it('keeps every status colour theme-aware together with its rgb triplet', () => {
    for (const name of STATUS_SLOTS) {
      expect(tokenValue(ROOT_VARS, `status-${name}`), name).toBeTruthy()
      expect(tokenValue(DARK_VARS, `status-${name}`), `${name} 暗色档`).toBeTruthy()
      expect(tokenValue(ROOT_VARS, `status-${name}-rgb`), `${name}-rgb`).toBeTruthy()
      expect(tokenValue(DARK_VARS, `status-${name}-rgb`), `${name}-rgb 暗色档`).toBeTruthy()
    }
  })

  // 深底子树（灯箱 / 视频 OSD）底色恒为近黑，必须把全部状态色与文本色固定为浅色档。
  // 只覆盖 danger 曾迫使各处改用具名色绕过规范；此断言防止机制再次残缺。
  it('overrides every status colour on constant-dark subtrees', () => {
    for (const name of STATUS_SLOTS) {
      expect(
        tokenValue(DARK_SURFACE_VARS, `status-${name}`),
        `.on-dark-surface 缺 --status-${name}`,
      ).toBeTruthy()
      expect(
        DARK_SURFACE_VARS,
        `.on-dark-surface 必须重绑 --color-status-${name}，否则子元素会继承 :root 已求值的亮色深色档`,
      ).toContain(`--color-status-${name}:`)
    }
    expect(
      tokenValue(DARK_SURFACE_VARS, 'text-primary'),
      '.on-dark-surface 缺 --text-primary',
    ).toBeTruthy()
  })

  // @theme 是语义 utility（text-status-success、bg-status-warning/10）的来源。
  it('exposes status colours through @theme so utilities and opacity variants exist', () => {
    for (const name of STATUS_SLOTS) {
      expect(THEME_VARS).toContain(`--color-status-${name}:`)
    }
  })

  // Canvas 2D 的 fillStyle/strokeStyle 不参与 CSS 级联，var() 是非法颜色串且会被
  // 静默忽略（绘制回退到上一个颜色）。该类赋值必须写字面量。
  it('never passes var() to canvas 2d colours, which cannot resolve them', () => {
    const offences = findOffences(
      (line) => /(fillStyle|strokeStyle|shadowColor)\s*=/.test(line) && line.includes('var(--'),
    )
    expect(offences).toEqual([])
  })

  // 语义重叠的 token 必须合并，否则同一含义会有两套取值、且容易漂移：
  // 旧 --accent-green / --accent-amber 在亮色主题下是 #10b981 / #d97706，
  // 对浅底仅 2.45:1 / 3.08:1（低于正文 4.5:1），且未在深底子树覆写。
  it('does not resurrect the merged --accent-green / --accent-amber tokens', () => {
    for (const name of ['accent-green', 'accent-amber']) {
      expect(CSS, `--${name} 已并入状态色，不应再定义`).not.toContain(`--${name}:`)
    }
    expect(findOffences((line) => /--accent-(green|amber)\b/.test(line))).toEqual([])
  })

  // 「实底 + 白字」必须用跨主题固定的 -solid 档：主题感知档在暗色主题下是浅色
  // （--status-warning #fbbf24 对白字仅 1.67:1），白字会不可读。
  //
  // 只检查同一个字符串字面量内的组合：跨行/跨变量的拼装（如把 fill 与 text-white
  // 分别写在 className 表达式两侧）静态不可靠，强行分析会引入大量误报，
  // 那种情况交由人工 review 与视觉验证覆盖。
  it('never pairs a theme-aware status fill with white foreground', () => {
    const FILL_PREFIX = '(?:^|[\\s"\'`])bg-'
    // 实底档形如 bg-status-warning-solid 或 bg-[var(--status-warning-solid)]，可带
    // /90 之类的不透明度后缀；-soft 是低位底，两者都不算「裸的主题感知档」。
    const solidOrSoft = new RegExp(
      `${FILL_PREFIX}(?:\\[var\\(--status-[a-z]+-(?:solid|soft)\\)\\]|status-[a-z]+-(?:solid|soft))`,
    )
    const bareThemeAware = new RegExp(
      `${FILL_PREFIX}(?:\\[var\\(--status-[a-z]+\\)\\]|status-[a-z]+)(?:\\/\\d+)?(?![\\w-])`,
    )
    const whiteForeground = /text-white|text-\[var\(--accent-contrast\)\]/

    const offences = findOffences((line) => {
      const literal = line.match(/'([^']*)'|"([^"]*)"|`([^`]*)`/)
      if (!literal) return false
      const body = literal[1] ?? literal[2] ?? literal[3] ?? ''
      return bareThemeAware.test(body) && !solidOrSoft.test(body) && whiteForeground.test(body)
    })
    expect(offences).toEqual([])
  })

  // 深底子树的文本三档必须保持「逐级变暗」的相对层级。
  // 曾经的问题：直接照搬 .dark 会让最低档 (#475569) 在视频底上掉到 2.55:1，
  // 比被替换的原始硬编码 (4.00) 更差；三档挤在一起也会让层级不可辨。
  it('keeps the dark-subtree text scale distinguishable', () => {
    // --video-surface #0b0e14
    const VIDEO_SURFACE = [11, 14, 20]

    /** WCAG 相对亮度 */
    const luminance = ([r, g, b]: number[]): number => {
      const channel = (value: number): number => {
        const v = value / 255
        return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
      }
      return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }
    const contrast = (hex: string, background: number[]): number => {
      const values = hex.replace('#', '').match(/../g) ?? []
      const l1 = luminance(values.map((h) => parseInt(h, 16)))
      const l2 = luminance(background)
      return (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05)
    }

    /** 取深底子树的文本档；缺失时让测试直接失败，而不是拿空串去算对比度 */
    const textLevel = (name: string): string => {
      const value = tokenValue(DARK_SURFACE_VARS, name)
      if (value === undefined) throw new Error(`.on-dark-surface 缺 --${name}`)
      return value
    }

    const levels = ['text-primary', 'text-secondary', 'text-muted'].map((name) => {
      const value = textLevel(name)
      return { name, value, ratio: contrast(value, VIDEO_SURFACE) }
    })

    for (let i = 1; i < levels.length; i += 1) {
      const brighter = levels[i - 1]
      const darker = levels[i]
      // 逐级变暗：primary 最亮，muted 最暗
      expect(
        darker.ratio,
        `${darker.name} (${darker.value}, ${darker.ratio.toFixed(2)}) 应比 ${brighter.name} 更暗`,
      ).toBeLessThan(brighter.ratio)

      // 仅「单调」不足以守住层级：三档整体上移（17.6 / 13.0 / 7.5）同样单调，
      // 但相邻档几乎重合，二级与三级文字在视觉上无法区分。要求相邻档拉开明确的
      // 对比度步长——1.5× 是启发式阈值，用于捕捉「层级塌陷」，不表达精确比例要求。
      const step = brighter.ratio / darker.ratio
      expect(
        step,
        `${brighter.name} (${brighter.ratio.toFixed(2)}) 与 ${darker.name} ` +
          `(${darker.ratio.toFixed(2)}) 仅相差 ${step.toFixed(2)}×，层级不可辨`,
      ).toBeGreaterThanOrEqual(1.5)
    }

    // 最低档不得低于被替换的原始硬编码 (zinc-500 在视频底上约 4.0:1)
    const muted = levels[2]
    expect(
      muted.ratio,
      `--text-muted ${muted.value} 在视频底上仅 ${muted.ratio.toFixed(2)}:1`,
    ).toBeGreaterThanOrEqual(4)
  })

  // 规范禁止硬编码具名色；唯一豁免是矢量/分类色板（需同容器内并列区分）。
  it('keeps hard-coded named colours confined to the sanctioned palette file', () => {
    const ALLOWED = 'features/tasks/components/rulesStudioTypes.ts'
    const NAMED_COLOUR =
      /(?:bg|text|border|from|to|via|ring|fill|stroke|divide|outline|caret|decoration|placeholder|accent|shadow)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}/

    const offences = findOffences((line) => NAMED_COLOUR.test(line)).filter(
      (location) => !location.startsWith(ALLOWED),
    )
    expect(offences).toEqual([])
  })
})
