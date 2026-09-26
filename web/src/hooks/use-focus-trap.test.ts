import { describe, expect, it } from 'vitest'
import { FOCUSABLE_SELECTOR } from './use-focus-trap'

describe('FOCUSABLE_SELECTOR', () => {
  it('covers the Tab-order controls that a modal must trap', () => {
    for (const fragment of ['a[href]', 'button:not([disabled])', 'input:not([disabled])']) {
      expect(FOCUSABLE_SELECTOR).toContain(fragment)
    }
  })

  it('excludes negative-tabindex elements so panels do not enter the cycle', () => {
    expect(FOCUSABLE_SELECTOR).toContain('[tabindex]:not([tabindex="-1"])')
  })
})
