import { describe, expect, it } from 'vitest'
import type { AlgorithmItem, AlgorithmVersionItem, Camera } from '@/types'
import {
  DEFAULT_ANALYSIS_FPS,
  buildQuickCreatePayload,
  pickActiveVersion,
  pickRecommendedAlgorithmId,
  splitCamerasByTask,
} from './taskDraft'

function camera(cameraId: string, name = cameraId): Camera {
  return {
    id: 1,
    cameraId,
    name,
    protocol: 'rtsp',
    rtspUrl: `rtsp://${cameraId}/main`,
    subRtspUrl: `rtsp://${cameraId}/sub`,
    streamMode: 'auto',
    remark: '',
    lastProbeStatus: 'healthy',
    lastCodec: 'h264',
    lastWidth: 1920,
    lastHeight: 1080,
    lastFps: 25,
    createdAt: 0,
    updatedAt: 0,
  }
}

function version(overrides: Partial<AlgorithmVersionItem> = {}): AlgorithmVersionItem {
  return {
    id: 1,
    algorithmId: 'general_detection',
    version: '1.0.0',
    platformId: 'rk3588',
    normalizedPlatformId: 'rk3588',
    compatibleWithHost: true,
    minAdapterVersion: '0.1.0',
    packageRoot: '/opt/algo',
    fpsTiers: [{ fps: 15, units: 100 }],
    configSchema: {},
    manifestRaw: {},
    packageSizeBytes: 1024,
    isActive: true,
    isBuiltin: true,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  }
}

function algorithm(
  algorithmId: string,
  options: { activeVersion?: string; versions?: AlgorithmVersionItem[] } = {},
): AlgorithmItem {
  return {
    id: 1,
    algorithmId,
    name: algorithmId,
    algorithmType: 'detection',
    alarmTypeId: 'intrusion',
    activeVersion: options.activeVersion ?? '1.0.0',
    description: '',
    isBuiltin: true,
    createdAt: 0,
    updatedAt: 0,
    versions: options.versions ?? [version({ algorithmId })],
  }
}

describe('splitCamerasByTask', () => {
  it('excludes configured channels and counts them', () => {
    const cameras = [camera('cam-1'), camera('cam-2'), camera('cam-3')]
    const { creatable, configuredCount } = splitCamerasByTask(cameras, new Set(['cam-2']))

    expect(creatable.map((item) => item.cameraId)).toEqual(['cam-1', 'cam-3'])
    expect(configuredCount).toBe(1)
  })

  it('keeps all channels creatable when no task exists', () => {
    const cameras = [camera('cam-1'), camera('cam-2')]
    const { creatable, configuredCount } = splitCamerasByTask(cameras, new Set())

    expect(creatable).toHaveLength(2)
    expect(configuredCount).toBe(0)
  })

  it('reports every channel as configured when all have tasks', () => {
    const cameras = [camera('cam-1'), camera('cam-2')]
    const { creatable, configuredCount } = splitCamerasByTask(cameras, new Set(['cam-1', 'cam-2']))

    expect(creatable).toEqual([])
    expect(configuredCount).toBe(2)
  })
})

describe('pickRecommendedAlgorithmId', () => {
  it('prefers general_detection over lexicographic order', () => {
    const items = [algorithm('aaa_detector'), algorithm('general_detection'), algorithm('zzz')]
    expect(pickRecommendedAlgorithmId(items)).toBe('general_detection')
  })

  it('falls back to the lexicographically first installed algorithm', () => {
    const items = [algorithm('zzz'), algorithm('bbb'), algorithm('aaa')]
    expect(pickRecommendedAlgorithmId(items)).toBe('aaa')
  })

  it('ignores algorithms without an installed version when others are installed', () => {
    const items = [algorithm('aaa_missing', { activeVersion: '' }), algorithm('bbb_installed')]
    expect(pickRecommendedAlgorithmId(items)).toBe('bbb_installed')
  })

  it('returns empty string when nothing is installed so the caller can block submit', () => {
    expect(pickRecommendedAlgorithmId([algorithm('aaa', { activeVersion: '' })])).toBe('')
    expect(pickRecommendedAlgorithmId([])).toBe('')
  })
})

describe('pickActiveVersion', () => {
  it('prefers the version flagged isActive', () => {
    const algo = algorithm('a', {
      activeVersion: '1.0.0',
      versions: [
        version({ version: '1.0.0', isActive: false }),
        version({ version: '2.0.0', isActive: true }),
      ],
    })
    expect(pickActiveVersion(algo)?.version).toBe('2.0.0')
  })

  it('falls back to the version matching activeVersion then the first entry', () => {
    const matched = algorithm('a', {
      activeVersion: '1.5.0',
      versions: [
        version({ version: '1.0.0', isActive: false }),
        version({ version: '1.5.0', isActive: false }),
      ],
    })
    expect(pickActiveVersion(matched)?.version).toBe('1.5.0')

    const first = algorithm('a', {
      activeVersion: '',
      versions: [version({ version: '0.9.0', isActive: false })],
    })
    expect(pickActiveVersion(first)?.version).toBe('0.9.0')
    expect(pickActiveVersion(undefined)).toBeUndefined()
  })
})

describe('buildQuickCreatePayload', () => {
  it('omits streamMode and motionGate so creation never rewrites camera-level config', () => {
    const payload = buildQuickCreatePayload({
      cameraId: 'cam-1',
      name: 'Task-cam-1',
      algorithmId: 'general_detection',
      algorithmVersion: version(),
      desiredEnabled: false,
    })

    expect('streamMode' in payload).toBe(false)
    expect('motionGate' in payload).toBe(false)
  })

  it('creates a single disabled instance as a draft when arming is off', () => {
    const payload = buildQuickCreatePayload({
      cameraId: 'cam-1',
      name: '  Task-cam-1  ',
      algorithmId: 'general_detection',
      algorithmVersion: version(),
      desiredEnabled: false,
    })

    expect(payload.name).toBe('Task-cam-1')
    expect(payload.rules).toEqual([])
    expect(payload.desiredEnabled).toBe(false)
    expect(payload.algorithmInstances).toHaveLength(1)
    expect(payload.algorithmInstances?.[0]).toMatchObject({
      algorithmId: 'general_detection',
      analysisFps: DEFAULT_ANALYSIS_FPS,
      enabled: false,
    })
  })

  it('selects all declared enum candidates when the schema omits a default', () => {
    const payload = buildQuickCreatePayload({
      cameraId: 'cam-1',
      name: 'Task-cam-1',
      algorithmId: 'general_detection',
      algorithmVersion: version({
        configSchema: {
          properties: {
            target_classes: { type: 'array', items: { enum: ['person', 'car'] } },
            allowed_classes: { type: 'array', items: { enum: ['helmet'] }, default: ['helmet'] },
          },
        },
      }),
      desiredEnabled: true,
    })

    expect(payload.algorithmInstances?.[0]?.algoParams).toEqual({
      target_classes: ['person', 'car'],
      allowed_classes: ['helmet'],
    })
  })

  it('never injects keys the algorithm schema does not declare', () => {
    const payload = buildQuickCreatePayload({
      cameraId: 'cam-1',
      name: 'Task-cam-1',
      algorithmId: 'general_detection',
      algorithmVersion: version({
        configSchema: { properties: { score_threshold: { type: 'number', default: 0.4 } } },
      }),
      desiredEnabled: true,
    })

    const params = payload.algorithmInstances?.[0]?.algoParams ?? {}
    expect(params).toEqual({ score_threshold: 0.4 })
    expect(params).not.toHaveProperty('confidence_threshold')
    expect(params).not.toHaveProperty('confidenceThreshold')
    expect(params).not.toHaveProperty('target_classes')
    expect(params).not.toHaveProperty('targetClasses')
  })

  it('builds empty params when the algorithm exposes no schema', () => {
    const payload = buildQuickCreatePayload({
      cameraId: 'cam-1',
      name: 'Task-cam-1',
      algorithmId: 'general_detection',
      algorithmVersion: undefined,
      desiredEnabled: true,
    })

    expect(payload.algorithmInstances?.[0]?.algoParams).toEqual({})
  })
  it('asserts the channel has no task yet so a concurrent create is rejected', () => {
    const payload = buildQuickCreatePayload({
      cameraId: 'cam-1',
      name: '  库房正门  ',
      algorithmId: 'general_detection',
      desiredEnabled: false,
    })

    expect(payload.configRevision).toBe(0)
  })
})
