import { useEffect, useState } from 'react'

/**
 * 基础值防抖 Hook
 *
 * 在指定的 delayMs 毫秒延迟后返回最新的输入值，在组件卸载或输入变化时自动重置定时器。
 */
export function useDebounce<T>(value: T, delayMs = 300): T {
  const [debouncedValue, setDebouncedValue] = useState<T>(value)

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedValue(value)
    }, delayMs)

    return () => {
      clearTimeout(timer)
    }
  }, [value, delayMs])

  return debouncedValue
}
