import { create } from 'zustand'

interface AuthState {
  token: string | null
  username: string | null
  /** 凭据签发时间（UTC 毫秒），用于按会话推导过期时刻；未登录为 null */
  issuedAt: number | null
  /** 凭据过期时刻（UTC 毫秒）；未登录为 null */
  expiresAt: number | null
  isAuthenticated: boolean
  login: (token: string, username: string, remember?: boolean, expiresAt?: number) => void
  logout: () => void
}

const STORAGE_KEY_TOKEN = 'heimdall-token'
const STORAGE_KEY_USER = 'heimdall-user'
const STORAGE_KEY_EXPIRES = 'heimdall-token-expires-at'
export const STORAGE_KEY_REMEMBER_USER = 'heimdall-remember-user'

/**
 * 「记住我」只持久化账号名，不持久化凭据。
 *
 * 凭据一律只写入 `sessionStorage`：`localStorage` 中的 Token 对同源脚本永久可读，
 * 任何一次 XSS 都能把它外带并在异地重放，而本系统的凭据本身就有数小时至一天级的寿命。
 * 关闭标签页即失效是这里愿意付出的代价。
 */
export function getRememberedUser(): string {
  if (typeof window !== 'undefined') {
    return window.localStorage?.getItem(STORAGE_KEY_REMEMBER_USER) || ''
  }
  return ''
}

function readSession(key: string): string | null {
  if (typeof window === 'undefined') return null
  return window.sessionStorage?.getItem(key) ?? null
}

function getInitialToken(): string | null {
  return readSession(STORAGE_KEY_TOKEN)
}

function getInitialUser(): string | null {
  return readSession(STORAGE_KEY_USER)
}

function getInitialExpiresAt(): number | null {
  const raw = readSession(STORAGE_KEY_EXPIRES)
  if (!raw) return null
  const parsed = Number.parseInt(raw, 10)
  return Number.isFinite(parsed) ? parsed : null
}

/**
 * 清理随版本演进遗留的持久化凭据。
 *
 * 早期版本把 Token 写进 `localStorage`。若不主动清除，升级后的浏览器里那份
 * 长效凭据仍会被旧路径读到或在后续请求中被继续携带。这里做一次性回收。
 */
function purgeLegacyPersistentCredentials(): void {
  if (typeof window === 'undefined') return
  window.localStorage?.removeItem(STORAGE_KEY_TOKEN)
  window.localStorage?.removeItem(STORAGE_KEY_USER)
  window.localStorage?.removeItem(STORAGE_KEY_EXPIRES)
}

function clearSessionCredentials(): void {
  if (typeof window === 'undefined') return
  window.sessionStorage?.removeItem(STORAGE_KEY_TOKEN)
  window.sessionStorage?.removeItem(STORAGE_KEY_USER)
  window.sessionStorage?.removeItem(STORAGE_KEY_EXPIRES)
}

export const useAuthStore = create<AuthState>((set) => {
  purgeLegacyPersistentCredentials()

  const initialToken = getInitialToken()
  const initialUser = getInitialUser()
  const initialExpiresAt = getInitialExpiresAt()
  // 会话恢复时同样要判过期：进程重启后 sessionStorage 仍在，
  // 此时带着一个已过期的 Token 进入工作区只会先吃一次 401 再被踢出。
  const stillValid = initialExpiresAt === null || initialExpiresAt > Date.now()

  if (initialToken && !stillValid) {
    clearSessionCredentials()
  }

  const token = stillValid ? initialToken : null
  const username = stillValid ? initialUser : null
  const expiresAt = stillValid ? initialExpiresAt : null

  return {
    token,
    username,
    issuedAt: null,
    expiresAt,
    isAuthenticated: Boolean(token),

    login: (newToken, username, remember = false, expiresAt) => {
      const validExpiresAt =
        typeof expiresAt === 'number' && Number.isFinite(expiresAt) ? expiresAt : null

      if (typeof window !== 'undefined') {
        window.sessionStorage?.setItem(STORAGE_KEY_TOKEN, newToken)
        window.sessionStorage?.setItem(STORAGE_KEY_USER, username)
        if (validExpiresAt !== null) {
          window.sessionStorage?.setItem(STORAGE_KEY_EXPIRES, String(validExpiresAt))
        } else {
          window.sessionStorage?.removeItem(STORAGE_KEY_EXPIRES)
        }

        // 账号名按「记住我」决定是否跨会话保留；凭据本身从不落 localStorage。
        if (remember) {
          window.localStorage?.setItem(STORAGE_KEY_REMEMBER_USER, username)
        } else {
          window.localStorage?.removeItem(STORAGE_KEY_REMEMBER_USER)
        }
        purgeLegacyPersistentCredentials()
      }
      set({
        token: newToken,
        username,
        issuedAt: Date.now(),
        expiresAt: validExpiresAt,
        isAuthenticated: true,
      })
    },

    logout: () => {
      clearSessionCredentials()
      // 保留 STORAGE_KEY_REMEMBER_USER：退出登录不应抹掉用户勾选的账号记忆，
      // 只有「登录时未勾选记住我」才清除（已在上面的 login 分支处理）。
      set({ token: null, username: null, issuedAt: null, expiresAt: null, isAuthenticated: false })
    },
  }
})
