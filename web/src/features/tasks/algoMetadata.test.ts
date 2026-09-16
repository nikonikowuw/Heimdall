import { describe, expect, it } from 'vitest'
import {
  buildSchemaDefaultParams,
  extractConfigProperties,
  extractTargetClasses,
} from './algoMetadata'

describe('algoMetadata', () => {
  it('reads target classes from the schema enum', () => {
    const classes = extractTargetClasses({
      configSchema: {
        properties: { target_classes: { items: { enum: ['person', 'car'] } } },
      },
      manifestRaw: {},
    })
    expect(classes).toEqual(['person', 'car'])
  })

  it('falls back to allowed_classes and classes aliases', () => {
    expect(
      extractTargetClasses({
        configSchema: { properties: { allowed_classes: { items: { enum: ['helmet'] } } } },
        manifestRaw: {},
      }),
    ).toEqual(['helmet'])
    expect(
      extractTargetClasses({
        configSchema: { properties: { classes: { items: { enum: ['fire'] } } } },
        manifestRaw: {},
      }),
    ).toEqual(['fire'])
  })

  it('falls back to manifestRaw.classes when the schema has no enum', () => {
    expect(
      extractTargetClasses({
        configSchema: { properties: {} },
        manifestRaw: { classes: ['person', 'bicycle'] },
      }),
    ).toEqual(['person', 'bicycle'])
  })

  it('ignores malformed values instead of throwing', () => {
    expect(extractTargetClasses(null)).toEqual([])
    expect(extractTargetClasses(undefined)).toEqual([])
    expect(
      extractTargetClasses({
        configSchema: 'not-an-object',
        manifestRaw: { classes: [1, 2] },
      }),
    ).toEqual([])
    expect(
      extractTargetClasses({
        configSchema: { properties: { target_classes: { items: { enum: [] } } } },
        manifestRaw: {},
      }),
    ).toEqual([])
  })

  it('returns a copy so callers cannot mutate the manifest', () => {
    const manifestRaw = { classes: ['person'] }
    const classes = extractTargetClasses({ configSchema: {}, manifestRaw })
    classes.push('car')
    expect(manifestRaw.classes).toEqual(['person'])
  })

  it('drops non-object property entries', () => {
    const props = extractConfigProperties({
      configSchema: { properties: { confidence_threshold: { type: 'number' }, broken: 42 } },
      manifestRaw: {},
    })
    expect(Object.keys(props)).toEqual(['confidence_threshold'])
  })
})

describe('buildSchemaDefaultParams', () => {
  it('prefers declared defaults and falls back to minimum for bare numbers', () => {
    expect(
      buildSchemaDefaultParams({
        configSchema: {
          properties: {
            score_threshold: { type: 'number', default: 0.4 },
            iou_threshold: { type: 'number', minimum: 0.2, maximum: 0.9 },
          },
        },
        manifestRaw: {},
      }),
    ).toEqual({ score_threshold: 0.4, iou_threshold: 0.2 })
  })

  it('defaults booleans to false, string enums to their first entry, and omits unknown types', () => {
    expect(
      buildSchemaDefaultParams({
        configSchema: {
          properties: {
            track_enabled: { type: 'boolean' },
            normalize: { type: 'string', enum: ['none', 'imagenet'] },
            opaque_handle: { type: 'object' },
          },
        },
        manifestRaw: {},
      }),
    ).toEqual({ track_enabled: false, normalize: 'none' })
  })

  it('returns empty params for a malformed or missing schema', () => {
    expect(buildSchemaDefaultParams(undefined)).toEqual({})
    expect(buildSchemaDefaultParams(null)).toEqual({})
    expect(buildSchemaDefaultParams({ configSchema: 'oops', manifestRaw: {} })).toEqual({})
  })
})
