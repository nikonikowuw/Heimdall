import { describe, expect, it } from 'vitest'
import { cn } from './utils'

describe('cn utility function', () => {
  it('should merge class names correctly', () => {
    const result = cn('bg-red-500', 'text-white', { 'p-4': true, 'p-2': false })
    expect(result).toBe('bg-red-500 text-white p-4')
  })

  it('should resolve tailwind conflict classes', () => {
    const result = cn('px-2', 'px-4')
    expect(result).toBe('px-4')
  })
})
