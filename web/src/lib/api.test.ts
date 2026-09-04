import { describe, expect, it, vi } from 'vitest'
import { authApi, ApiError } from './api'
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
})
