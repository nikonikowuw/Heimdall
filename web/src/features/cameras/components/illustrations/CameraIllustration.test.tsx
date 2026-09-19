import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import {
  CameraIllustration,
  DomeCameraIllustration,
  BulletCameraIllustration,
  PtzCameraIllustration,
} from './CameraIllustration'
import { resolveCameraModelType } from './cameraModelType'

describe('resolveCameraModelType', () => {
  it('respects explicit type override', () => {
    expect(resolveCameraModelType({ name: '标准枪机' }, 'dome')).toBe('dome')
    expect(resolveCameraModelType({ name: '半球监控' }, 'bullet')).toBe('bullet')
    expect(resolveCameraModelType({ name: '枪机' }, 'ptz')).toBe('ptz')
  })

  it('detects ptz cameras only when explicit keywords match', () => {
    expect(resolveCameraModelType({ name: '广场 360 度 PTZ 球机' })).toBe('ptz')
    expect(resolveCameraModelType({ remark: '高点全景云台监控' })).toBe('ptz')
    expect(resolveCameraModelType({ rtspUrl: 'rtsp://10.0.0.1/speed-dome/1' })).toBe('ptz')
  })

  it('detects bullet cameras from keywords', () => {
    expect(resolveCameraModelType({ name: '东区周界枪机 01' })).toBe('bullet')
    expect(resolveCameraModelType({ remark: '出入口筒机' })).toBe('bullet')
    expect(resolveCameraModelType({ rtspUrl: 'rtsp://10.0.0.2/bullet_sub' })).toBe('bullet')
  })

  it('detects dome cameras from keywords', () => {
    expect(resolveCameraModelType({ name: '1F 电梯间半球' })).toBe('dome')
    expect(resolveCameraModelType({ remark: '走廊吸顶监控' })).toBe('dome')
    expect(resolveCameraModelType({ rtspUrl: 'rtsp://10.0.0.3/dome-stream' })).toBe('dome')
  })

  it('defaults to standard bullet camera when no keywords match (never randomly assigns PTZ)', () => {
    const res1 = resolveCameraModelType({ cameraId: 'cam-alpha-001', name: '网络摄像头 1' })
    const res2 = resolveCameraModelType({ cameraId: 'cam-beta-002', name: '仓库主通道' })
    expect(res1).toBe('bullet')
    expect(res2).toBe('bullet')
  })

  it('returns bullet when camera is null or empty', () => {
    expect(resolveCameraModelType(null)).toBe('bullet')
    expect(resolveCameraModelType({})).toBe('bullet')
  })
})

describe('Camera Illustrations Component Suite', () => {
  it('renders DomeCameraIllustration with correct accessibility and state', () => {
    const htmlOnline = renderToString(<DomeCameraIllustration status="online" aiActive={false} />)
    expect(htmlOnline).toContain('aria-label="Dome Camera Illustration (online)"')

    const htmlWarningAi = renderToString(
      <DomeCameraIllustration status="warning" aiActive={true} />,
    )
    expect(htmlWarningAi).toContain('aria-label="Dome Camera Illustration (warning, AI Active)"')
  })

  it('renders BulletCameraIllustration with correct accessibility and state', () => {
    const htmlOffline = renderToString(
      <BulletCameraIllustration status="offline" aiActive={false} />,
    )
    expect(htmlOffline).toContain('aria-label="Bullet Camera Illustration (offline)"')

    const htmlOnlineAi = renderToString(
      <BulletCameraIllustration status="online" aiActive={true} />,
    )
    expect(htmlOnlineAi).toContain('aria-label="Bullet Camera Illustration (online, AI Active)"')
  })

  it('renders PtzCameraIllustration with correct accessibility and state', () => {
    const htmlPtz = renderToString(<PtzCameraIllustration status="online" aiActive={true} />)
    expect(htmlPtz).toContain('aria-label="PTZ Camera Illustration (online, AI Active)"')
  })

  it('CameraIllustration dispatcher correctly delegates to requested type', () => {
    const htmlBullet = renderToString(
      <CameraIllustration type="bullet" status="online" aiActive={false} />,
    )
    expect(htmlBullet).toContain('Bullet Camera Illustration')

    const htmlPtz = renderToString(
      <CameraIllustration type="ptz" status="warning" aiActive={true} />,
    )
    expect(htmlPtz).toContain('PTZ Camera Illustration')

    const htmlDome = renderToString(
      <CameraIllustration type="dome" status="offline" aiActive={false} />,
    )
    expect(htmlDome).toContain('Dome Camera Illustration')
  })
})
