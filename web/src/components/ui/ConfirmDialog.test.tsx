import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { ConfirmDialog } from './ConfirmDialog'

function render(props: Partial<Parameters<typeof ConfirmDialog>[0]> = {}): string {
  return renderToString(
    <ConfirmDialog
      isOpen
      title="Delete camera"
      description="This cannot be undone"
      confirmLabel="Delete"
      cancelLabel="Cancel"
      onConfirm={() => {}}
      onClose={() => {}}
      {...props}
    />,
  )
}

describe('ConfirmDialog', () => {
  it('exposes a labelled alertdialog for danger confirmations', () => {
    const html = render()

    expect(html).toContain('role="alertdialog"')
    expect(html).toContain('aria-modal="true"')
    expect(html).toContain('aria-labelledby=')
    expect(html).toContain('aria-describedby=')
    expect(html).toContain('Delete')
    expect(html).toContain('Cancel')
  })

  it('uses dialog role for warning variant so it is not announced as an error', () => {
    const html = render({ variant: 'warning' })

    expect(html).toContain('role="dialog"')
    expect(html).not.toContain('role="alertdialog"')
  })

  it('surfaces the failure reason without a second notification channel', () => {
    const html = render({ errorMessage: 'Camera is busy' })

    expect(html).toContain('role="alert"')
    expect(html).toContain('Camera is busy')
  })

  it('omits the error region when there is no failure', () => {
    expect(render()).not.toContain('role="alert"')
  })

  it('marks the confirm action as the initial focus target', () => {
    expect(render()).toContain('data-autofocus')
  })

  it('prevents all actions during submission without dropping the focused button', () => {
    const html = render({ isConfirming: true })
    const disabledButtons = html.match(/<button\b[^>]*aria-disabled="true"[^>]*>/g) ?? []

    expect(disabledButtons).toHaveLength(3)
    expect(disabledButtons.every((button) => !/\sdisabled(?:=|\s|>)/.test(button))).toBe(true)
    expect(html).toContain('animate-spin')
  })

  it('renders caller content for objects that require manual verification', () => {
    const html = render({ children: <p>alice · #7</p> })

    expect(html).toContain('alice · #7')
  })
})
