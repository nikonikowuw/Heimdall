import { describe, expect, it, vi } from 'vitest'
import { authApi, cameraApi, ApiError } from './api'
import { useAuthStore } from '../stores/auth'

describe('API Client', () => {
  it('ApiError should hold code and message', () => {
    const err = new ApiError('密码错误', 10007)
    expect(err.message).toBe('密码错误')
    expect(err.code).toBe(10007)
    expect(err.name).toBe('ApiError')
  })

  it('getInitStatus should return parsed data', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: { initialized: true },
        timestamp: 1747584000000,
      }),
    })

    const res = await authApi.getInitStatus()
    expect(res.initialized).toBe(true)
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/auth/init-status',
      expect.objectContaining({ method: 'GET' }),
    )
  })

  it('login should include body and return access token', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: {
          accessToken: 'jwt-token-xyz',
          username: 'admin',
          expiresAt: 1747670400000,
        },
        timestamp: 1747584000000,
      }),
    })

    const res = await authApi.login({ username: 'admin', password: 'password123' })
    expect(res.accessToken).toBe('jwt-token-xyz')
    expect(res.username).toBe('admin')
  })

  it('401 response should trigger local logout', async () => {
    useAuthStore.getState().login('existing-token', 'admin')
    expect(useAuthStore.getState().isAuthenticated).toBe(true)

    global.fetch = vi.fn().mockResolvedValue({
      status: 401,
      json: async () => ({
        code: 10003,
        message: '凭据无效或已被撤销',
        data: null,
        timestamp: 1747584000000,
      }),
    })

    await expect(authApi.getMe()).rejects.toThrow('凭据无效或已被撤销')
    expect(useAuthStore.getState().isAuthenticated).toBe(false)
  })

  it('should inject Accept-Language header into request', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: { initialized: true },
        timestamp: 1747584000000,
      }),
    })

    await authApi.getInitStatus()
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/auth/init-status',
      expect.objectContaining({
        headers: expect.objectContaining({
          'Accept-Language': expect.any(String),
        }),
      }),
    )
  })

  it('cameraApi.list should fetch cameras list', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: [
          {
            id: 1,
            cameraId: 'cam-01',
            name: 'Gate Cam',
            protocol: 'rtsp',
            rtspUrl: 'rtsp://127.0.0.1:8554/live',
            subRtspUrl: '',
            remark: '',
            lastProbeStatus: 'healthy',
            lastCodec: 'h264',
            lastWidth: 1920,
            lastHeight: 1080,
            lastFps: 25.0,
            createdAt: 1747584000000,
            updatedAt: 1747584000000,
          },
        ],
        timestamp: 1747584000000,
      }),
    })

    const list = await cameraApi.list()
    expect(list).toHaveLength(1)
    expect(list[0].cameraId).toBe('cam-01')
  })

  it('cameraApi.negotiateWhep should handle SDP offer/answer with Location header', async () => {
    const mockHeaders = new Headers({
      Location: '/api/v1/webrtc/whep/session-1234',
    })

    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 201,
      headers: mockHeaders,
      text: async () => 'v=0\r\no=- 123 2 IN IP4 127.0.0.1\r\ns=-\r\n',
    })

    const res = await cameraApi.negotiateWhep('cam-01', 'v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\n')
    expect(res.answerSdp).toContain('v=0')
    expect(res.location).toBe('/api/v1/webrtc/whep/session-1234')
  })

  it('cameraApi.closeWhep should send DELETE request', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      ok: true,
      status: 204,
    })

    await cameraApi.closeWhep('/api/v1/webrtc/whep/session-1234')
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/webrtc/whep/session-1234',
      expect.objectContaining({ method: 'DELETE' }),
    )
  })
})
