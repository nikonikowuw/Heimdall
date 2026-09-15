export type NumericParamType = 'number' | 'integer'

export interface NumericParamConfig {
  type: NumericParamType
  minimum?: number
  maximum?: number
  defaultValue: number
}

function asFiniteNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

export function getNumericParamConfig(
  property: Record<string, unknown>,
): NumericParamConfig | null {
  const type = property.type
  if (type !== 'number' && type !== 'integer') return null

  const minimum = asFiniteNumber(property.minimum)
  const maximum = asFiniteNumber(property.maximum)
  const defaultValue = asFiniteNumber(property.default) ?? minimum ?? 0

  return { type, minimum, maximum, defaultValue }
}

/** Keep intermediate keyboard states out of numeric model updates. */
export function parseNumericDraft(draft: string, type: NumericParamType): number | null {
  const value = draft.trim()
  if (
    !value ||
    value === '-' ||
    value === '+' ||
    value === '.' ||
    value === '-.' ||
    value === '+.'
  ) {
    return null
  }

  if (type === 'integer' && !/^[+-]?\d+$/.test(value)) {
    return null
  }

  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : null
}

export function clampNumericParam(value: number, minimum?: number, maximum?: number): number {
  const withMinimum = minimum === undefined ? value : Math.max(minimum, value)
  return maximum === undefined ? withMinimum : Math.min(maximum, withMinimum)
}

export function formatNumericDraft(value: number, type: NumericParamType): string {
  return type === 'integer' ? String(Math.round(value)) : String(value)
}

export function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}
