import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import type { CaptureRecord } from '@/types'
import { CaptureCardItem } from './CapturesContent'

vi.mock('@/lib/api', () => ({
  evidenceApi: {
    getImageUrl: (relPath: string) => `/api/v1/evidence/image/${relPath}`,
  },
}))

const t = (key: string): string => key

function makeCapture(overrides: Partial<CaptureRecord> = {}): CaptureRecord {
  return {
    id: 1,
    captureId: 'cap_1',
    cameraId: 'cam_1',
    trackId: 42,
    targetLabel: 'person',
    confidence: 0.9,
    qualityScore: 0.7,
    bboxJson: '{}',
    imageId: 'img_1',
    imageRelPath: 'cam_1/full.jpg',
    cropImageId: '',
    cropImageRelPath: '',
    bodyCropImageId: 'body_1',
    bodyCropImageRelPath: 'cam_1/body.jpg',
    capturedAt: 1_741_100_060_000,
    createdAt: 1_741_100_060_000,
    ...overrides,
  }
}

describe('CaptureCardItem 抓拍卡片', () => {
  it('renders the body crop as the thumbnail so clothing stays identifiable', () => {
    const html = renderToString(
      <CaptureCardItem capture={makeCapture()} onSelect={() => {}} t={t} />,
    )
    expect(html).toContain('cam_1/body.jpg')
    expect(html).not.toContain('cam_1/full.jpg"')
  })

  it('falls back to the face crop and then the panorama for legacy rows', () => {
    const faceOnly = renderToString(
      <CaptureCardItem
        capture={makeCapture({ bodyCropImageRelPath: '', cropImageRelPath: 'cam_1/face.jpg' })}
        onSelect={() => {}}
        t={t}
      />,
    )
    expect(faceOnly).toContain('cam_1/face.jpg')

    const legacy = renderToString(
      <CaptureCardItem
        capture={makeCapture({
          bodyCropImageRelPath: '',
          cropImageRelPath: '',
        })}
        onSelect={() => {}}
        t={t}
      />,
    )
    expect(legacy).toContain('cam_1/full.jpg')
  })

  it('renders the track badge as a disabled button when no track filter is wired up', () => {
    const html = renderToString(
      <CaptureCardItem capture={makeCapture()} onSelect={() => {}} t={t} />,
    )
    expect(html).toContain('#<!-- -->42')
    expect(html).toContain('disabled')
  })

  it('keeps the track badge clickable when a track filter callback is provided', () => {
    const html = renderToString(
      <CaptureCardItem
        capture={makeCapture()}
        onSelect={() => {}}
        onSelectTrack={() => {}}
        t={t}
      />,
    )
    expect(html).not.toContain('disabled')
    expect(html).toContain('cursor-pointer hover:bg-cyan-400/90')
  })
})
