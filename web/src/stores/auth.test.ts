import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { getRememberedUser, useAuthStore } from './auth'

describe('Auth store', () => {
  const createMockStorage = () => {
    const store: Record<string, string> = {}
    return {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, value: string) => {
        store[key] = String(value)
      },
      removeItem: (key: string) => {
        delete store[key]
      },
      clear: () => {
        for (const k of Object.keys(store)) delete store[k]
      },
      key: () => null,
      length: 0,
    } as unknown as Storage
  }

  beforeEach(() => {
    const localStorage = createMockStorage()
    const sessionStorage = createMockStorage()
    ;(
      globalThis as unknown as { window: { localStorage: Storage; sessionStorage: Storage } }
    ).window = {
      localStorage,
      sessionStorage,
    }
  })

  afterEach(() => {
    useAuthStore.getState().logout()
  })

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

  it('should handle remember me functionality correctly', () => {
    const store = useAuthStore.getState()

    // 勾选记住我登录
    store.login('token-remember', 'commander_neo', true)
    expect(getRememberedUser()).toBe('commander_neo')

    // 登出后用户名依然保留在记住我的持久化中
    store.logout()
    expect(getRememberedUser()).toBe('commander_neo')

    // 未勾选记住我登录
    store.login('token-temporary', 'guest_operator', false)
    expect(getRememberedUser()).toBe('')

    store.logout()
    expect(getRememberedUser()).toBe('')
  })
})
