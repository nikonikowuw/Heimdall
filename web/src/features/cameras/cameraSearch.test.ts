import { describe, expect, it } from 'vitest'
import {
  matchesCameraAddressQuery,
  matchesCameraNameQuery,
  normalizeCameraSearchQuery,
} from './cameraSearch'

const camera = {
  name: '1',
  cameraId: 'cam-001',
  rtspUrl: 'rtsp://admin:secret@192.168.1.100:554/Streaming/Channels/101',
  subRtspUrl: 'rtsp://admin:secret@192.168.1.100:554/Streaming/Channels/102',
  gb28181DeviceId: null,
  gb28181ChannelId: null,
}

describe('camera search fields', () => {
  it('normalizes queries by trimming and lowercasing', () => {
    expect(normalizeCameraSearchQuery('  Gate East 01  ')).toBe('gate east 01')
    expect(normalizeCameraSearchQuery('   ')).toBe('')
  })

  it('matches a numeric device name without searching its stream URL', () => {
    expect(matchesCameraNameQuery(camera, '1')).toBe(true)
    expect(matchesCameraNameQuery(camera, '192.168.1.100')).toBe(false)
  })

  it('supports pre-normalized queries for loop optimizations', () => {
    const preNormalized = normalizeCameraSearchQuery('  FRONT ')
    expect(matchesCameraNameQuery({ name: 'Front Gate' }, '', preNormalized)).toBe(true)
    expect(matchesCameraAddressQuery(camera, '', 'cam-001')).toBe(true)
    expect(matchesCameraAddressQuery(camera, '', 'unknown-id')).toBe(false)
  })

  it('matches main and sub stream URLs and keeps IDs searchable in the address field', () => {
    expect(matchesCameraAddressQuery(camera, '192.168.1.100')).toBe(true)
    expect(matchesCameraAddressQuery(camera, '/channels/102')).toBe(true)
    expect(matchesCameraAddressQuery(camera, 'cam-001')).toBe(true)
    expect(matchesCameraAddressQuery({ ...camera, subRtspUrl: '' }, '/channels/102')).toBe(false)
  })

  it('matches GB28181 device and channel identifiers', () => {
    const gbCamera = {
      ...camera,
      gb28181DeviceId: '34020000001320000001',
      gb28181ChannelId: '34020000001320000002',
    }
    expect(matchesCameraAddressQuery(gbCamera, '1320000002')).toBe(true)
  })

  it('trims and folds case, and treats an empty query as no filter', () => {
    expect(matchesCameraNameQuery({ name: 'Front Gate' }, '  FRONT ')).toBe(true)
    expect(matchesCameraNameQuery({ name: 'Front Gate' }, '  ')).toBe(true)
    expect(matchesCameraAddressQuery(camera, '  ')).toBe(true)
  })
})
