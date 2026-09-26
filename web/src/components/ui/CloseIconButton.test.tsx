import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { CloseIconButton } from './CloseIconButton'
import { FormErrorAlert } from './FormErrorAlert'

describe('CloseIconButton', () => {
  it('exposes the localized accessible name', () => {
    const html = renderToString(<CloseIconButton onClick={() => {}} label="关闭" />)

    expect(html).toContain('aria-label="关闭"')
    expect(html).toContain('type="button"')
  })

  it('reflects the disabled state while a request is in flight', () => {
    const html = renderToString(<CloseIconButton onClick={() => {}} label="Close" disabled />)

    expect(html).toContain('disabled')
  })

  it('can prevent activation without removing the control from the focus order', () => {
    const html = renderToString(<CloseIconButton onClick={() => {}} label="Close" ariaDisabled />)

    expect(html).toContain('aria-disabled="true"')
    expect(html).not.toMatch(/<button[^>]*\sdisabled(?:=|\s|>)/)
  })

  it('keeps the shared 36px hit target across variants', () => {
    for (const variant of ['plain', 'surface'] as const) {
      expect(
        renderToString(<CloseIconButton onClick={() => {}} label="x" variant={variant} />),
      ).toContain('h-9 w-9')
    }
  })
})

describe('FormErrorAlert', () => {
  it('announces the failure as an alert', () => {
    const html = renderToString(<FormErrorAlert message="删除失败" />)

    expect(html).toContain('role="alert"')
    expect(html).toContain('删除失败')
  })

  it('renders nothing when there is no message', () => {
    expect(renderToString(<FormErrorAlert message={null} />)).toBe('')
    expect(renderToString(<FormErrorAlert message={undefined} />)).toBe('')
    expect(renderToString(<FormErrorAlert message="" />)).toBe('')
  })
})
