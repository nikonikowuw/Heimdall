import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import '@/i18n'
import { RefreshButton } from './RefreshButton'

describe('RefreshButton', () => {
  it('renders icon-only page action button with aria-label', () => {
    const html = renderToString(<RefreshButton onClick={() => {}} />)

    expect(html).toContain('page-action-btn')
    expect(html).toContain('page-action-btn--icon')
    expect(html).toContain('aria-label=')
    expect(html).toContain('aria-hidden="true"')
  })

  it('renders spin animation and disabled state when loading', () => {
    const html = renderToString(<RefreshButton onClick={() => {}} loading />)

    expect(html).toContain('animate-spin')
    expect(html).toContain('disabled=""')
  })

  it('accepts custom label and className', () => {
    const html = renderToString(
      <RefreshButton onClick={() => {}} label="刷新当前列表" className="reticle-target" />,
    )

    expect(html).toContain('aria-label="刷新当前列表"')
    expect(html).toContain('title="刷新当前列表"')
    expect(html).toContain('reticle-target')
  })
})
