import { describe, expect, it } from 'vitest'
import { useAuthStore } from './auth'

describe('Auth store', () => {
  it('should handle login and logout correctly', () => {
    const store = useAuthStore.getState()
    expect(store.isAuthenticated).toBe(false)

    // 执行登录
    store.login('test-token-123', 'admin')
    const loggedInState = useAuthStore.getState()
    expect(loggedInState.isAuthenticated).toBe(true)
    expect(loggedInState.token).toBe('test-token-123')
    expect(loggedInState.username).toBe('admin')

    // 执行登出
    store.logout()
    const loggedOutState = useAuthStore.getState()
    expect(loggedOutState.isAuthenticated).toBe(false)
    expect(loggedOutState.token).toBeNull()
  })
})
