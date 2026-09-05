import i18n from '../i18n'
import { useAuthStore } from '../stores/auth'
import type {
  AdminUserDto,
  AlarmRecord,
  AlgoManifest,
  ApiResponse,
  Camera,
  CaptureRecord,
  ChangePasswordRequest,
  CreateCameraRequest,
  InitStatusResponse,
  InitializeRequest,
  LoginRequest,
  LoginResponse,
  RecognitionRecord,
  SandboxCheckResult,
  TaskConfigDto,
  TaskSummaryDto,
  UpdateCameraRequest,
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

export async function request<T>(endpoint: string, options: RequestInit = {}): Promise<T> {
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

export const api = {
  get<T>(endpoint: string): Promise<T> {
    return request<T>(endpoint, { method: 'GET' })
  },
  post<T>(endpoint: string, data?: unknown): Promise<T> {
    return request<T>(endpoint, {
      method: 'POST',
      body: data !== undefined ? JSON.stringify(data) : undefined,
    })
  },
  put<T>(endpoint: string, data?: unknown): Promise<T> {
    return request<T>(endpoint, {
      method: 'PUT',
      body: data !== undefined ? JSON.stringify(data) : undefined,
    })
  },
  delete<T>(endpoint: string): Promise<T> {
    return request<T>(endpoint, { method: 'DELETE' })
  },
}

export const cameraApi = {
  list(): Promise<Camera[]> {
    return api.get<Camera[]>('/cameras')
  },

  get(id: string): Promise<Camera> {
    return api.get<Camera>(`/cameras/${encodeURIComponent(id)}`)
  },

  create(data: CreateCameraRequest): Promise<Camera> {
    return api.post<Camera>('/cameras', data)
  },

  update(id: string, data: UpdateCameraRequest): Promise<Camera> {
    return api.put<Camera>(`/cameras/${encodeURIComponent(id)}`, data)
  },

  delete(id: string): Promise<void> {
    return api.delete<void>(`/cameras/${encodeURIComponent(id)}`)
  },

  probe(id: string): Promise<{ codec: string; width: number; height: number; fps: number }> {
    return api.post<{ codec: string; width: number; height: number; fps: number }>(
      `/cameras/${encodeURIComponent(id)}/probe`,
    )
  },

  deduceSubStream(rtspUrl: string): Promise<import('../types').SubStreamCandidate[]> {
    return api.post<import('../types').SubStreamCandidate[]>('/cameras/deduce-substream', {
      rtspUrl,
    })
  },

  getLiveStreamUrl(cameraId: string, stream: 'main' | 'sub' = 'main'): string {
    const token = useAuthStore.getState().token || ''
    const path = `${BASE_URL}/live/${encodeURIComponent(cameraId)}.flv?stream=${stream}&token=${encodeURIComponent(token)}`
    if (typeof window !== 'undefined' && window.location?.origin) {
      return new URL(path, window.location.origin).href
    }
    return path
  },

  getWebCodecsWsUrl(cameraId: string, stream: 'main' | 'sub' = 'main'): string {
    const token = useAuthStore.getState().token || ''
    const isSsl = typeof window !== 'undefined' && window.location?.protocol === 'https:'
    const protocol = isSsl ? 'wss:' : 'ws:'
    const host =
      typeof window !== 'undefined' && window.location?.host
        ? window.location.host
        : 'localhost:8080'
    return `${protocol}//${host}${BASE_URL}/live/${encodeURIComponent(cameraId)}/webcodecs?stream=${stream}&token=${encodeURIComponent(token)}`
  },
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

function toQueryString(params?: Record<string, string | number | undefined>): string {
  if (!params) return ''
  const searchParams = new URLSearchParams()
  for (const [key, val] of Object.entries(params)) {
    if (val !== undefined && val !== null && val !== '') {
      searchParams.set(key, String(val))
    }
  }
  const str = searchParams.toString()
  return str ? `?${str}` : ''
}

export const taskApi = {
  getTask(cameraId: string): Promise<TaskConfigDto> {
    return api.get<TaskConfigDto>(`/tasks/${encodeURIComponent(cameraId)}`)
  },

  updateTask(cameraId: string, data: TaskConfigDto): Promise<TaskConfigDto> {
    return api.put<TaskConfigDto>(`/tasks/${encodeURIComponent(cameraId)}`, data)
  },

  list(): Promise<TaskSummaryDto[]> {
    return api.get<TaskSummaryDto[]>('/tasks')
  },
}

export const alarmApi = {
  list(params?: {
    cameraId?: string
    status?: string
    startTime?: number
    endTime?: number
    limit?: number
    offset?: number
  }): Promise<AlarmRecord[]> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      status: params?.status,
      start_time: params?.startTime,
      end_time: params?.endTime,
      limit: params?.limit,
      offset: params?.offset,
    })
    return api.get<AlarmRecord[]>(`/alarms${qs}`)
  },

  updateStatus(id: number, status: string): Promise<AlarmRecord> {
    return api.put<AlarmRecord>(`/alarms/${id}/status`, { status })
  },
}

export const evidenceApi = {
  listCaptures(params?: {
    cameraId?: string
    targetLabel?: string
    startTime?: number
    endTime?: number
    limit?: number
    offset?: number
  }): Promise<CaptureRecord[]> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      target_label: params?.targetLabel,
      start_time: params?.startTime,
      end_time: params?.endTime,
      limit: params?.limit,
      offset: params?.offset,
    })
    return api.get<CaptureRecord[]>(`/evidence/captures${qs}`)
  },

  listRecognitions(params?: {
    cameraId?: string
    limit?: number
    offset?: number
  }): Promise<RecognitionRecord[]> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      limit: params?.limit,
      offset: params?.offset,
    })
    return api.get<RecognitionRecord[]>(`/evidence/recognitions${qs}`)
  },

  getImageUrl(relPath: string): string {
    if (!relPath) return ''
    const token = useAuthStore.getState().token
    const q = token ? `?token=${encodeURIComponent(token)}` : ''
    return `${BASE_URL}/evidence/image/${relPath}${q}`
  },
}

export const algoApi = {
  listPackages(): Promise<AlgoManifest[]> {
    return api.get<AlgoManifest[]>('/algo/packages')
  },

  verifyPackage(packagePath?: string): Promise<SandboxCheckResult> {
    return api.post<SandboxCheckResult>('/algo/verify', { packagePath })
  },

  uploadPackage(file: File): Promise<SandboxCheckResult> {
    const formData = new FormData()
    formData.append('file', file)
    const token = useAuthStore.getState().token
    return fetch(`${BASE_URL}/algo/upload`, {
      method: 'POST',
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      body: formData,
    }).then(async (res) => {
      const data = await res.json()
      if (!res.ok || data.code !== 0) {
        throw new Error(data.message || 'Failed to upload algorithm package')
      }
      return data.data
    })
  },

  scanPackages(): Promise<number> {
    return api.post<number>('/algo/scan', {})
  },
}
