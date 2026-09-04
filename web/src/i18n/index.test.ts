import { describe, expect, it } from 'vitest'
import i18n, { SUPPORTED_LOCALES } from './index'

describe('i18n configuration', () => {
  it('should have supported locales defined', () => {
    expect(SUPPORTED_LOCALES).toContain('zh-CN')
    expect(SUPPORTED_LOCALES).toContain('zh-TW')
    expect(SUPPORTED_LOCALES).toContain('en')
  })

  it('should initialize and support language switching', async () => {
    await i18n.changeLanguage('en')
    expect(i18n.language).toBe('en')
    await i18n.changeLanguage('zh-CN')
    expect(i18n.language).toBe('zh-CN')
  })
})
