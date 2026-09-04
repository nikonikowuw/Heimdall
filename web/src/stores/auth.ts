import { create } from 'zustand'

interface AuthState {
  token: string | null
  username: string | null
  isAuthenticated: boolean
  login: (token: string, username: string, remember?: boolean) => void
  logout: () => void
}

const STORAGE_KEY_TOKEN = 'argus-token'
const STORAGE_KEY_USER = 'argus-user'

function getInitialToken(): string | null {
  if (typeof window !== 'undefined') {
    return (
      window.localStorage?.getItem(STORAGE_KEY_TOKEN) ||
      window.sessionStorage?.getItem(STORAGE_KEY_TOKEN) ||
      null
    )
  }
  return null
}

function getInitialUser(): string | null {
  if (typeof window !== 'undefined') {
    return (
      window.localStorage?.getItem(STORAGE_KEY_USER) ||
      window.sessionStorage?.getItem(STORAGE_KEY_USER) ||
      null
    )
  }
  return null
}

export const useAuthStore = create<AuthState>((set) => {
  const initialToken = getInitialToken()
  const initialUser = getInitialUser()

  return {
    token: initialToken,
    username: initialUser,
    isAuthenticated: Boolean(initialToken),

    login: (token: string, username: string, remember = true) => {
      if (typeof window !== 'undefined') {
        if (remember) {
          window.localStorage?.setItem(STORAGE_KEY_TOKEN, token)
          window.localStorage?.setItem(STORAGE_KEY_USER, username)
          window.sessionStorage?.removeItem(STORAGE_KEY_TOKEN)
          window.sessionStorage?.removeItem(STORAGE_KEY_USER)
        } else {
          window.sessionStorage?.setItem(STORAGE_KEY_TOKEN, token)
          window.sessionStorage?.setItem(STORAGE_KEY_USER, username)
          window.localStorage?.removeItem(STORAGE_KEY_TOKEN)
          window.localStorage?.removeItem(STORAGE_KEY_USER)
        }
      }
      set({ token, username, isAuthenticated: true })
    },

    logout: () => {
      if (typeof window !== 'undefined') {
        window.localStorage?.removeItem(STORAGE_KEY_TOKEN)
        window.localStorage?.removeItem(STORAGE_KEY_USER)
        window.sessionStorage?.removeItem(STORAGE_KEY_TOKEN)
        window.sessionStorage?.removeItem(STORAGE_KEY_USER)
      }
      set({ token: null, username: null, isAuthenticated: false })
    },
  }
})
