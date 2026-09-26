import { describe, expect, it } from 'vitest'
import { renderToString } from 'react-dom/server'
import '@/i18n'
import { SearchInput } from './SearchInput'

describe('SearchInput', () => {
  it('renders default search field with search icon and input', () => {
    const html = renderToString(
      <SearchInput value="" onChange={() => {}} placeholder="搜索关键字..." />,
    )

    expect(html).toContain('search-field')
    expect(html).toContain('search-field__icon')
    expect(html).toContain('search-field__input')
    expect(html).toContain('placeholder="搜索关键字..."')
    expect(html).toContain('aria-label="搜索关键字..."')
  })

  it('renders keyboard hint when showKbdHint is true and value is empty', () => {
    const html = renderToString(
      <SearchInput value="" onChange={() => {}} showKbdHint data-search-input="true" />,
    )

    expect(html).toContain('search-field__hint')
    expect(html).toContain('data-search-input="true"')
    expect(html).not.toContain('search-field__clear')
  })

  it('renders clear button with aria-label when value is present and onClear is provided', () => {
    const html = renderToString(
      <SearchInput
        value="test query"
        onChange={() => {}}
        onClear={() => {}}
        clearAriaLabel="清空当前搜索"
        showKbdHint
      />,
    )

    expect(html).toContain('search-field__clear')
    expect(html).toContain('aria-label="清空当前搜索"')
    expect(html).toContain('title="清空当前搜索"')
    expect(html).not.toContain('search-field__hint')
  })

  it('applies compact and form size variants properly', () => {
    const compactHtml = renderToString(
      <SearchInput value="" onChange={() => {}} sizeVariant="compact" />,
    )
    expect(compactHtml).toContain('search-field--compact')

    const formHtml = renderToString(<SearchInput value="" onChange={() => {}} sizeVariant="form" />)
    expect(formHtml).toContain('search-field--form')
  })

  it('supports custom clearButtonClassName and containerClassName', () => {
    const html = renderToString(
      <SearchInput
        value="query"
        onChange={() => {}}
        onClear={() => {}}
        containerClassName="custom-container"
        clearButtonClassName="reticle-target"
      />,
    )

    expect(html).toContain('custom-container')
    expect(html).toContain('reticle-target')
  })
})
