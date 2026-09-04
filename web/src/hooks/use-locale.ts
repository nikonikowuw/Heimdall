import { useCallback, useSyncExternalStore } from 'react'
import i18n, { type Locale, SUPPORTED_LOCALES } from '../i18n'

function subscribe(callback: () => void) {
  i18n.on('languageChanged', callback)
  return () => {
    i18n.off('languageChanged', callback)
  }
}

function getSnapshot(): Locale {
  const current = i18n.language as Locale
  return SUPPORTED_LOCALES.includes(current) ? current : 'en'
}

export function useLocale() {
  const locale = useSyncExternalStore(subscribe, getSnapshot, () => 'zh-CN' as Locale)

  const setLocale = useCallback((nextLocale: Locale) => {
    i18n.changeLanguage(nextLocale).then(() => {
      if (typeof window !== 'undefined' && typeof window.localStorage !== 'undefined') {
        window.localStorage.setItem('argus-locale', nextLocale)
      }
    })
  }, [])

  return {
    locale,
    setLocale,
    supportedLocales: SUPPORTED_LOCALES,
  }
}
