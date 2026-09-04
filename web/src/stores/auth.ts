import { create } from 'zustand'

interface AuthState {
  token: string | null
  username: string | null
  isAuthenticated: boolean
  login: (token: string, username: string) => void
  logout: () => void
}

const STORAGE_KEY_TOKEN = 'argus-token'
const STORAGE_KEY_USER = 'argus-user'

function getInitialToken(): string | null {
  if (typeof window !== 'undefined' && typeof window.localStorage !== 'undefined') {
    return window.localStorage.getItem(STORAGE_KEY_TOKEN)
  }
  return null
}

function getInitialUser(): string | null {
  if (typeof window !== 'undefined' && typeof window.localStorage !== 'undefined') {
    return window.localStorage.getItem(STORAGE_KEY_USER)
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

    login: (token: string, username: string) => {
      if (typeof window !== 'undefined' && typeof window.localStorage !== 'undefined') {
        window.localStorage.setItem(STORAGE_KEY_TOKEN, token)
        window.localStorage.setItem(STORAGE_KEY_USER, username)
      }
      set({ token, username, isAuthenticated: true })
    },

    logout: () => {
      if (typeof window !== 'undefined' && typeof window.localStorage !== 'undefined') {
        window.localStorage.removeItem(STORAGE_KEY_TOKEN)
        window.localStorage.removeItem(STORAGE_KEY_USER)
      }
      set({ token: null, username: null, isAuthenticated: false })
    },
  }
})
