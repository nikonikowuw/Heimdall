import React, { useEffect, useRef, useState } from 'react'
import { Check, ChevronDown, Globe } from 'lucide-react'
import { useLocale } from '../hooks/use-locale'
import { type Locale, SUPPORTED_LOCALES } from '../i18n'

const LOCALE_LABELS: Record<Locale, string> = {
  'zh-CN': '简体中文',
  'zh-TW': '繁體中文',
  en: 'English',
}

interface LocaleDropdownProps {
  placement?: 'bottom-right' | 'top-right' | 'right-bottom'
  compact?: boolean
  variant?: 'pill' | 'icon'
}

export const LocaleDropdown: React.FC<LocaleDropdownProps> = ({
  placement = 'bottom-right',
  compact = false,
  variant = 'pill',
}) => {
  const { locale, setLocale } = useLocale()
  const [open, setOpen] = useState(false)
  const containerRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false)
      }
    }

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setOpen(false)
      }
    }

    if (open) {
      document.addEventListener('mousedown', handleClickOutside)
      document.addEventListener('keydown', handleKeyDown)
    }

    return () => {
      document.removeEventListener('mousedown', handleClickOutside)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [open])

  // 根据 placement 决定浮层定位样式
  const getDropdownPosClasses = () => {
    switch (placement) {
      case 'top-right':
        return 'bottom-full right-0 mb-2'
      case 'right-bottom':
        return 'left-full bottom-0 ml-3'
      case 'bottom-right':
      default:
        return 'top-full right-0 mt-2'
    }
  }

  const shortLabel = locale === 'zh-CN' ? '简' : locale === 'zh-TW' ? '繁' : 'EN'

  return (
    <div ref={containerRef} className="relative inline-block select-none">
      {/* 触发按钮 */}
      {variant === 'icon' ? (
        <button
          type="button"
          onClick={() => setOpen((prev) => !prev)}
          aria-expanded={open}
          aria-haspopup="listbox"
          title={`当前语言: ${LOCALE_LABELS[locale]} (点击切换)`}
          className={`flex h-10 w-10 flex-col items-center justify-center rounded-xl text-[var(--text-secondary)] transition-colors hover:bg-[var(--accent-soft)] focus:outline-none ${
            open ? 'bg-[var(--accent-soft)] text-[var(--accent)] ring-1 ring-[var(--ring)]' : ''
          }`}
        >
          <Globe className="h-4 w-4" />
          <span className="font-mono text-[9px] font-bold uppercase">{shortLabel}</span>
        </button>
      ) : (
        <button
          type="button"
          onClick={() => setOpen((prev) => !prev)}
          aria-expanded={open}
          aria-haspopup="listbox"
          title="选择界面语言 / Select Language"
          className={`flex items-center gap-1.5 rounded-full border border-black/5 bg-white/60 px-3 py-1 font-mono text-[10px] font-bold text-slate-700 shadow-sm backdrop-blur-md transition-all hover:bg-[var(--accent-soft)] focus:outline-none dark:border-white/10 dark:bg-black/40 dark:text-slate-300 ${
            open ? 'ring-2 ring-[var(--ring)]' : ''
          }`}
        >
          <Globe className="h-3.5 w-3.5 text-[var(--accent)]" />
          <span>{compact ? shortLabel : LOCALE_LABELS[locale]}</span>
          <ChevronDown
            className={`h-3 w-3 text-slate-400 transition-transform duration-200 ${
              open ? 'rotate-180' : ''
            }`}
          />
        </button>
      )}

      {/* 下拉列表面板 */}
      {open && (
        <div
          role="listbox"
          className={`lens-glass animate-in fade-in zoom-in-95 absolute z-50 min-w-[130px] overflow-hidden rounded-2xl border border-[var(--border)] p-1.5 shadow-2xl backdrop-blur-2xl transition-all duration-200 ${getDropdownPosClasses()}`}
        >
          <div className="flex flex-col gap-0.5">
            {SUPPORTED_LOCALES.map((loc) => {
              const isActive = locale === loc
              return (
                <button
                  key={loc}
                  type="button"
                  role="option"
                  aria-selected={isActive}
                  onClick={() => {
                    setLocale(loc)
                    setOpen(false)
                  }}
                  className={`flex w-full items-center justify-between rounded-xl px-3 py-2 text-xs font-medium transition-colors ${
                    isActive
                      ? 'bg-[var(--accent)] text-white shadow-sm'
                      : 'text-[var(--text-secondary)] hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)]'
                  }`}
                >
                  <span className="font-sans">{LOCALE_LABELS[loc]}</span>
                  {isActive && <Check className="h-3.5 w-3.5 stroke-[2.5]" />}
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
