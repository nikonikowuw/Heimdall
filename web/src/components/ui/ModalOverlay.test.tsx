import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ModalOverlay } from './ModalOverlay'

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
