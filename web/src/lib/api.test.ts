import { describe, expect, it, vi } from 'vitest'
import { authApi, algorithmApi, cameraApi, taskApi, oplogApi, ApiError } from './api'
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

  it('upload should preserve plain-text gateway errors', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 413,
      statusText: 'Payload Too Large',
      text: async () => 'gateway rejected upload',
    })

    const file = new Blob(['payload'], { type: 'application/gzip' }) as File
    await expect(algorithmApi.uploadPackage(file)).rejects.toThrow('gateway rejected upload')
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

  it('oplogApi.list should pass module pagination and abort signal', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: [],
        timestamp: 1747584000000,
      }),
    })
    const controller = new AbortController()

    await expect(
      oplogApi.list({ module: 'camera', limit: 50, offset: 100 }, controller.signal),
    ).resolves.toEqual([])
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/logs/operations?module=camera&limit=50&offset=100',
      expect.objectContaining({ method: 'GET', signal: controller.signal }),
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

  it('cameraApi.create should send POST request with camera payload', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: {
          id: 2,
          cameraId: 'cam-02',
          name: 'North Yard',
          rtspUrl: 'rtsp://10.0.0.2:554/live',
        },
        timestamp: 1747584000000,
      }),
    })

    const created = await cameraApi.create({
      name: 'North Yard',
      rtspUrl: 'rtsp://10.0.0.2:554/live',
    })
    expect(created.cameraId).toBe('cam-02')
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/cameras',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({
          name: 'North Yard',
          rtspUrl: 'rtsp://10.0.0.2:554/live',
        }),
      }),
    )
  })

  it('cameraApi.update should send PUT request with encoded cameraId', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: {
          id: 1,
          cameraId: 'cam/01',
          name: 'Updated Cam',
          rtspUrl: 'rtsp://10.0.0.1:554/live2',
        },
        timestamp: 1747584000000,
      }),
    })

    const updated = await cameraApi.update('cam/01', { name: 'Updated Cam' })
    expect(updated.name).toBe('Updated Cam')
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/cameras/cam%2F01',
      expect.objectContaining({
        method: 'PUT',
        body: JSON.stringify({ name: 'Updated Cam' }),
      }),
    )
  })

  it('cameraApi.delete should send DELETE request with encoded cameraId', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: null,
        timestamp: 1747584000000,
      }),
    })

    await cameraApi.delete('cam/01')
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/cameras/cam%2F01',
      expect.objectContaining({
        method: 'DELETE',
      }),
    )
  })

  it('cameraApi.getLiveStreamUrl should build correct HTTP-FLV stream URL', () => {
    const url = cameraApi.getLiveStreamUrl('cam-01', 'main')
    expect(url).toContain('/api/v1/live/cam-01.flv?stream=main')
    expect(url).not.toContain('audio=true')
  })

  it('cameraApi.getLiveStreamUrl should append audio=true when audio is enabled', () => {
    const url = cameraApi.getLiveStreamUrl('cam-01', 'main', true)
    expect(url).toContain('/api/v1/live/cam-01.flv?')
    expect(url).toContain('stream=main')
    expect(url).toContain('audio=true')
  })

  it('cameraApi.getLiveStreamUrl should append video=false for an audio-only FLV stream', () => {
    const url = cameraApi.getLiveStreamUrl('cam-01', 'main', true, false)
    expect(url).toContain('audio=true')
    expect(url).toContain('video=false')
  })

  it('cameraApi.getWebCodecsWsUrl should build correct WebSocket WebCodecs URL', () => {
    const url = cameraApi.getWebCodecsWsUrl('cam-01', 'sub')
    expect(url).toContain('/api/v1/live/cam-01/webcodecs?stream=sub')
  })

  it('taskApi.deleteTask should send DELETE request with encoded cameraId', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: null,
        timestamp: 1747584000000,
      }),
    })

    await taskApi.deleteTask('cam/01')
    expect(global.fetch).toHaveBeenCalledWith(
      '/api/v1/tasks/cam%2F01',
      expect.objectContaining({
        method: 'DELETE',
      }),
    )
  })

  it('taskApi.list should fetch task summary list', async () => {
    global.fetch = vi.fn().mockResolvedValue({
      status: 200,
      json: async () => ({
        code: 0,
        message: 'success',
        data: [
          {
            cameraId: 'cam-01',
            name: 'Perimeter',
            desiredEnabled: true,
            rulesCount: 2,
            motionGateEnabled: true,
            updatedAt: 1747584000000,
          },
        ],
        timestamp: 1747584000000,
      }),
    })

    const tasks = await taskApi.list()
    expect(tasks).toHaveLength(1)
    expect(tasks[0].cameraId).toBe('cam-01')
    expect(tasks[0].rulesCount).toBe(2)
  })
})
