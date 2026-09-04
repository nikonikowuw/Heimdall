import i18n from '../i18n'
import { useAuthStore } from '../stores/auth'
import type {
  AdminUserDto,
  ApiResponse,
  ChangePasswordRequest,
  InitStatusResponse,
  InitializeRequest,
  LoginRequest,
  LoginResponse,
} from '../types'

export class ApiError extends Error {
  code: number
  constructor(message: string, code: number) {
    super(message)
    this.name = 'ApiError'
    this.code = code
  }
}

const BASE_URL = '/api/v1'

async function request<T>(endpoint: string, options: RequestInit = {}): Promise<T> {
  const token = useAuthStore.getState().token

  const lang = (i18n && i18n.language) || 'zh-CN'
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    'Accept-Language': lang,
    ...(options.headers as Record<string, string>),
  }

  if (token) {
    headers.Authorization = `Bearer ${token}`
  }

  const response = await fetch(`${BASE_URL}${endpoint}`, {
    ...options,
    headers,
  })

  // 如果遇到 401 Unauthorized 且非登录/初始化接口，触发本地登出
  if (
    response.status === 401 &&
    !endpoint.startsWith('/auth/login') &&
    !endpoint.startsWith('/auth/init')
  ) {
    useAuthStore.getState().logout()
  }

  const payload: ApiResponse<T> = await response.json().catch(() => {
    throw new ApiError('网络响应解析失败', response.status)
  })

  if (payload.code !== 0) {
    throw new ApiError(payload.message || '请求处理失败', payload.code)
  }

  return payload.data
}

export const authApi = {
  getInitStatus(): Promise<InitStatusResponse> {
    return request<InitStatusResponse>('/auth/init-status', { method: 'GET' })
  },

  initialize(data: InitializeRequest): Promise<LoginResponse> {
    return request<LoginResponse>('/auth/initialize', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  },

  login(data: LoginRequest): Promise<LoginResponse> {
    return request<LoginResponse>('/auth/login', {
      method: 'POST',
      body: JSON.stringify(data),
    })
  },

  changePassword(data: ChangePasswordRequest): Promise<AdminUserDto> {
    return request<AdminUserDto>('/auth/password', {
      method: 'PUT',
      body: JSON.stringify(data),
    })
  },

  getMe(): Promise<AdminUserDto> {
    return request<AdminUserDto>('/auth/me', { method: 'GET' })
  },

  logout(): Promise<void> {
    return request<void>('/auth/logout', { method: 'POST' })
  },
}
