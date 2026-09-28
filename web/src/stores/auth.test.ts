import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { STORAGE_KEY_REMEMBER_USER, getRememberedUser, useAuthStore } from './auth'

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
      /** 测试观察用：读取原始键集合 */
      __keys: () => Object.keys(store),
    } as unknown as Storage
  }

  let localStorage: Storage
  let sessionStorage: Storage

  beforeEach(() => {
    localStorage = createMockStorage()
    sessionStorage = createMockStorage()
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

    store.login('test-token-123', 'admin')
    const loggedInState = useAuthStore.getState()
    expect(loggedInState.isAuthenticated).toBe(true)
    expect(loggedInState.token).toBe('test-token-123')
    expect(loggedInState.username).toBe('admin')

    store.logout()
    const loggedOutState = useAuthStore.getState()
    expect(loggedOutState.isAuthenticated).toBe(false)
    expect(loggedOutState.token).toBeNull()
  })

  /**
   * 凭据只能存在于 sessionStorage。
   *
   * 这是安全边界而非实现细节：localStorage 对同源脚本永久可读，
   * 凭据一旦落在那里，任何一次 XSS 都能把它外带并在异地重放。
   */
  it('never persists credentials to localStorage', () => {
    useAuthStore.getState().login('secret-token', 'admin', true, Date.now() + 3_600_000)

    const persisted = (localStorage as unknown as { __keys: () => string[] }).__keys()
    expect(persisted).not.toContain('heimdall-token')
    expect(persisted).not.toContain('heimdall-user')
    // 「记住我」只保留账号名，供下次登录预填
    expect(persisted).toContain(STORAGE_KEY_REMEMBER_USER)
    expect(localStorage.getItem(STORAGE_KEY_REMEMBER_USER)).toBe('admin')

    expect(sessionStorage.getItem('heimdall-token')).toBe('secret-token')
  })

  it('should handle remember me functionality correctly', () => {
    const store = useAuthStore.getState()

    // 勾选记住我：账号名跨会话保留，凭据仍只在 sessionStorage
    store.login('token-remember', 'commander_neo', true)
    expect(getRememberedUser()).toBe('commander_neo')

    // 登出后账号名依然保留，方便下次预填
    store.logout()
    expect(getRememberedUser()).toBe('commander_neo')

    // 未勾选记住我：账号名被清除
    store.login('token-temporary', 'guest_operator', false)
    expect(getRememberedUser()).toBe('')

    store.logout()
    expect(getRememberedUser()).toBe('')
  })

  /** 服务端下发的过期时刻必须被记录并在会话恢复时用于判定。 */
  it('records expiresAt from the issued credential', () => {
    const expiresAt = Date.now() + 3_600_000
    useAuthStore.getState().login('token-with-expiry', 'admin', false, expiresAt)

    const state = useAuthStore.getState()
    expect(state.expiresAt).toBe(expiresAt)
    expect(state.issuedAt).toBeGreaterThan(0)
    expect(sessionStorage.getItem('heimdall-token-expires-at')).toBe(String(expiresAt))
  })

  /**
   * 遗留的持久化凭据必须被一次性回收。
   *
   * 早期版本把 Token 写进 localStorage；升级后若不清除，那份长效凭据仍会在
   * 旧代码路径或在后续请求中被读到，等于绕过了本次收敛。
   */
  it('purges legacy localStorage credentials on load', async () => {
    localStorage.setItem('heimdall-token', 'legacy-long-lived-token')
    localStorage.setItem('heimdall-user', 'legacy-admin')

    // 重置模块注册表，让下一次导入重新执行 store 的初始化路径
    vi.resetModules()
    const fresh = await import('./auth')
    fresh.useAuthStore.getState()

    expect(localStorage.getItem('heimdall-token')).toBeNull()
    expect(localStorage.getItem('heimdall-user')).toBeNull()
    expect(fresh.useAuthStore.getState().token).toBeNull()
  })
})
