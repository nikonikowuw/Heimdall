import i18n from 'i18next'
import resourcesToBackend from 'i18next-resources-to-backend'
import { initReactI18next } from 'react-i18next'

export const SUPPORTED_LOCALES = ['zh-CN', 'zh-TW', 'en'] as const
export type Locale = (typeof SUPPORTED_LOCALES)[number]

export const LOCALE_STORAGE_KEY = 'argus-locale'

function getInitialLocale(): Locale {
  if (typeof window !== 'undefined' && typeof window.localStorage !== 'undefined') {
    const saved = window.localStorage.getItem(LOCALE_STORAGE_KEY) as Locale
    if (saved && SUPPORTED_LOCALES.includes(saved)) {
      return saved
    }
  }
  return 'zh-CN'
}

i18n
  .use(
    resourcesToBackend(
      (language: string, namespace: string) => import(`./${language}/${namespace}.json`),
    ),
  )
  .use(initReactI18next)
  .init({
    lng: getInitialLocale(),
    fallbackLng: 'en',
    defaultNS: 'common',
    ns: ['common', 'camera', 'alarm', 'task', 'oplog', 'auth'],
    interpolation: {
      escapeValue: false,
    },
  })

export default i18n
