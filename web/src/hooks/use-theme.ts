import { useCallback, useEffect, useState } from 'react'

export type ThemeMode = 'light' | 'dark'

const THEME_STORAGE_KEY = 'argus_theme'

export function getInitialTheme(): ThemeMode {
  if (typeof window === 'undefined') return 'dark'
  const saved = localStorage.getItem(THEME_STORAGE_KEY)
  if (saved === 'light' || saved === 'dark') {
    return saved
  }
  // 默认暗色系统（更符合 AI 边缘计算与深空黑洞美学）
  return 'dark'
}

export function applyTheme(theme: ThemeMode) {
  if (typeof document === 'undefined') return
  if (theme === 'dark') {
    document.documentElement.classList.add('dark')
  } else {
    document.documentElement.classList.remove('dark')
  }
  localStorage.setItem(THEME_STORAGE_KEY, theme)
}

export function useTheme() {
  const [theme, setThemeState] = useState<ThemeMode>(() => {
    const init = getInitialTheme()
    applyTheme(init)
    return init
  })

  const isDark = theme === 'dark'

  const setTheme = useCallback((nextTheme: ThemeMode) => {
    setThemeState(nextTheme)
    applyTheme(nextTheme)
  }, [])

  const toggleTheme = useCallback(() => {
    setThemeState((prev) => {
      const next = prev === 'dark' ? 'light' : 'dark'
      applyTheme(next)
      return next
    })
  }, [])

  useEffect(() => {
    // 确保 html class 同步
    applyTheme(theme)
  }, [theme])

  return {
    theme,
    isDark,
    setTheme,
    toggleTheme,
  }
}
