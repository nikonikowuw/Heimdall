import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ModalOverlay } from './ModalOverlay'

const CSS = readFileSync(join(process.cwd(), 'src/styles/globals.css'), 'utf8')

const TIER_CLASSES = [
  'modal-surface--small',
  'modal-surface--compact',
  'modal-surface--narrow',
  'modal-surface--xl',
  'modal-surface--medium',
  'modal-surface--wide',
  'modal-surface--extra-wide',
] as const

function renderOverlay(surface?: 'glass' | 'solid'): string {
  return renderToString(
    <ModalOverlay
      isOpen
      onClose={() => {}}
      ariaLabel="Test modal"
      surface={surface}
      panelClassName="modal-surface--form"
    >
      <p>Modal content</p>
    </ModalOverlay>,
  )
}

describe('ModalOverlay surface', () => {
  it('keeps glass as the default surface', () => {
    expect(renderOverlay()).toContain('modal-surface--glass')
  })

  it('supports solid form surfaces without glass', () => {
    const html = renderOverlay('solid')

    expect(html).toContain('modal-surface--form')
    expect(html).not.toContain('modal-surface--glass')
  })
})

describe('ModalOverlay width contract', () => {
  // 历史缺陷：本组件曾为 modal 变体无条件注入 modal-surface--wide。同一元素带上两个
  // 同层同权重的档位时，只能靠规则源序决出胜负，导致 --small / --compact / --narrow
  // 调用点被静默改成 48rem。以下两条契约锁死该缺陷不再回潮。
  it('does not inject a width tier of its own', () => {
    // 省略 panelClassName 是旧实现注入 --wide 兜底的情形，必须保持无档位
    const html = renderToString(
      <ModalOverlay isOpen onClose={() => {}} ariaLabel="Test modal">
        <p>Modal content</p>
      </ModalOverlay>,
    )

    for (const tier of TIER_CLASSES) {
      expect(html, tier).not.toContain(tier)
    }
  })

  it('keeps every tier at its own unique max-width so source order cannot decide', () => {
    const widths = TIER_CLASSES.map((tier) => {
      const escaped = tier.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
      const rule = CSS.match(new RegExp(`\\n\\s*\\.${escaped}\\s*\\{([^}]*)\\}`))?.[1] ?? ''
      return { tier, width: rule.match(/max-width:\s*([^;]+);/)?.[1]?.trim() ?? '' }
    })

    for (const { tier, width } of widths) {
      expect(width, tier).not.toBe('')
    }
    const unique = new Set(widths.map((w) => w.width))
    expect(unique.size).toBe(widths.length)
  })
})
