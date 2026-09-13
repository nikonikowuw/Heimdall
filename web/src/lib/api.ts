import i18n from '../i18n'
import { useAuthStore } from '../stores/auth'
import type {
  AdminUserDto,
  AlarmRecord,
  AlarmStatus,
  AlgorithmInstanceDto,
  AlgorithmItem,
  AlgorithmStats,
  AlgorithmVersionItem,
  ApiResponse,
  BatchImportGbChannelsRequest,
  BatchImportGbChannelsResponse,
  Camera,
  CaptureRecord,
  ChangePasswordRequest,
  CreateAlgorithmInstanceRequest,
  CreateCameraRequest,
  DiscoveredDevice,
  Gb28181ConfigResponse,
  Gb28181Device,
  InitStatusResponse,
  InitializeRequest,
  LoginRequest,
  LoginResponse,
  OperationLog,
  PaginatedAlgorithms,
  PersonnelDetail,
  PersonnelItem,
  PersonnelStats,
  RecognitionRecord,
  SysGb28181Config,
  TaskConfigDto,
  TaskSummaryDto,
  UpdateAlgorithmInstanceRequest,
  UpdateCameraRequest,
  UpdateGb28181ConfigRequest,
  UploadAlgorithmResponse,
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

  getLiveStreamUrl(
    cameraId: string,
    stream: 'main' | 'sub' = 'main',
    audio = false,
    video = true,
  ): string {
    const token = useAuthStore.getState().token || ''
    const params = new URLSearchParams({ stream, token })
    if (audio) {
      params.set('audio', 'true')
    }
    if (!video) {
      params.set('video', 'false')
    }
    const path = `${BASE_URL}/live/${encodeURIComponent(cameraId)}.flv?${params.toString()}`
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

  getMe(signal?: AbortSignal): Promise<AdminUserDto> {
    return request<AdminUserDto>('/auth/me', { method: 'GET', signal })
  },

  logout(): Promise<void> {
    return request<void>('/auth/logout', { method: 'POST' })
  },
}

function toQueryString(params?: Record<string, string | number | boolean | undefined>): string {
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

export const oplogApi = {
  list(
    params?: { module?: string; limit?: number; offset?: number },
    signal?: AbortSignal,
  ): Promise<OperationLog[]> {
    const qs = toQueryString(params)
    return request<OperationLog[]>(`/logs/operations${qs}`, { method: 'GET', signal })
  },
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

  deleteTask(cameraId: string): Promise<void> {
    return api.delete<void>(`/tasks/${encodeURIComponent(cameraId)}`)
  },
}

export const alarmApi = {
  list(params?: {
    cameraId?: string
    status?: AlarmStatus | string
    targetLabel?: string
    ruleType?: string
    severity?: string
    startTime?: number
    endTime?: number
    limit?: number
    offset?: number
  }): Promise<AlarmRecord[]> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      status: params?.status,
      target_label: params?.targetLabel,
      rule_type: params?.ruleType,
      severity: params?.severity,
      start_time: params?.startTime,
      end_time: params?.endTime,
      limit: params?.limit,
      offset: params?.offset,
    })
    return api.get<AlarmRecord[]>(`/alarms${qs}`)
  },

  count(params?: {
    cameraId?: string
    status?: AlarmStatus | string
    targetLabel?: string
    ruleType?: string
    severity?: string
    startTime?: number
    endTime?: number
  }): Promise<{ total: number }> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      status: params?.status,
      target_label: params?.targetLabel,
      rule_type: params?.ruleType,
      severity: params?.severity,
      start_time: params?.startTime,
      end_time: params?.endTime,
    })
    return api.get<{ total: number }>(`/alarms/count${qs}`)
  },

  updateStatus(id: number, status: AlarmStatus): Promise<AlarmRecord> {
    return api.put<AlarmRecord>(`/alarms/${id}/status`, { status })
  },

  batchUpdateStatus(ids: number[], status: AlarmStatus): Promise<AlarmRecord[]> {
    return api.post<AlarmRecord[]>(`/alarms/batch-status`, { ids, status })
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

  countCaptures(params?: {
    cameraId?: string
    targetLabel?: string
    startTime?: number
    endTime?: number
  }): Promise<{ total: number }> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      target_label: params?.targetLabel,
      start_time: params?.startTime,
      end_time: params?.endTime,
    })
    return api.get<{ total: number }>(`/evidence/captures/count${qs}`)
  },

  listRecognitions(params?: {
    cameraId?: string
    status?: string
    startTime?: number
    endTime?: number
    limit?: number
    offset?: number
  }): Promise<RecognitionRecord[]> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      status: params?.status,
      start_time: params?.startTime,
      end_time: params?.endTime,
      limit: params?.limit,
      offset: params?.offset,
    })
    return api.get<RecognitionRecord[]>(`/evidence/recognitions${qs}`)
  },

  countRecognitions(params?: {
    cameraId?: string
    status?: string
    startTime?: number
    endTime?: number
  }): Promise<{ total: number }> {
    const qs = toQueryString({
      camera_id: params?.cameraId,
      status: params?.status,
      start_time: params?.startTime,
      end_time: params?.endTime,
    })
    return api.get<{ total: number }>(`/evidence/recognitions/count${qs}`)
  },

  reviewRecognition(
    recognitionId: string,
    data: {
      status: 'confirmed' | 'rejected'
      subjectId?: string
      subjectName?: string
      photoRelPath?: string
      similarity?: number
    },
  ): Promise<RecognitionRecord> {
    return api.post<RecognitionRecord>(`/evidence/recognitions/${recognitionId}/review`, data)
  },

  getImageUrl(relPath: string): string {
    if (!relPath) return ''
    const token = useAuthStore.getState().token
    const q = token ? `?token=${encodeURIComponent(token)}` : ''
    return `${BASE_URL}/evidence/image/${relPath}${q}`
  },
}

export const personnelApi = {
  list(params?: {
    keyword?: string
    limit?: number
    offset?: number
  }): Promise<{ items: PersonnelItem[]; total: number }> {
    const qs = toQueryString({
      keyword: params?.keyword,
      limit: params?.limit,
      offset: params?.offset,
    })
    return api.get<{ items: PersonnelItem[]; total: number }>(`/personnel${qs}`)
  },

  getStats(): Promise<PersonnelStats> {
    return api.get<PersonnelStats>('/personnel/stats')
  },

  getDetail(subjectId: string): Promise<PersonnelDetail> {
    return api.get<PersonnelDetail>(`/personnel/${encodeURIComponent(subjectId)}`)
  },

  create(formData: FormData): Promise<PersonnelDetail> {
    return postFormData<PersonnelDetail>('/personnel', formData)
  },

  update(
    subjectId: string,
    payload: { name?: string; idCard?: string; remark?: string },
  ): Promise<PersonnelDetail> {
    return api.put<PersonnelDetail>(`/personnel/${encodeURIComponent(subjectId)}`, payload)
  },

  delete(subjectId: string): Promise<{ subjectId: string; deleted: boolean }> {
    return api.delete<{ subjectId: string; deleted: boolean }>(
      `/personnel/${encodeURIComponent(subjectId)}`,
    )
  },

  addFaces(subjectId: string, formData: FormData): Promise<PersonnelDetail> {
    return postFormData<PersonnelDetail>(
      `/personnel/${encodeURIComponent(subjectId)}/faces`,
      formData,
    )
  },

  deleteFace(subjectId: string, faceId: string): Promise<PersonnelDetail> {
    return api.delete<PersonnelDetail>(
      `/personnel/${encodeURIComponent(subjectId)}/faces/${encodeURIComponent(faceId)}`,
    )
  },

  setPrimaryFace(subjectId: string, faceId: string): Promise<PersonnelDetail> {
    return api.put<PersonnelDetail>(
      `/personnel/${encodeURIComponent(subjectId)}/faces/${encodeURIComponent(faceId)}/primary`,
      {},
    )
  },
}

function postFormData<T>(endpoint: string, formData: FormData): Promise<T> {
  const token = useAuthStore.getState().token
  return fetch(`${BASE_URL}${endpoint}`, {
    method: 'POST',
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    body: formData,
  }).then(async (res) => {
    const raw = await res.text().catch(() => '')
    let data: { code: number; message?: string; data: T }
    try {
      // 网络响应只在 API 边界解析一次，避免先 json() 后 text() 消费同一个 body。
      data = JSON.parse(raw) as { code: number; message?: string; data: T }
    } catch {
      throw new Error(raw || `Request failed with status ${res.status} (${res.statusText})`)
    }
    if (!res.ok || data.code !== 0) {
      throw new Error(data.message || 'Request failed')
    }
    return data.data
  })
}

function uploadFormData<T>(endpoint: string, file: File): Promise<T> {
  const formData = new FormData()
  formData.append('file', file)
  return postFormData<T>(endpoint, formData)
}

export const algorithmApi = {
  list(params?: {
    page?: number
    pageSize?: number
    keyword?: string
    algorithmType?: string
    isBuiltin?: boolean
  }): Promise<PaginatedAlgorithms> {
    const qs = toQueryString(params)
    return api.get<PaginatedAlgorithms>(`/algorithms${qs}`)
  },

  getStats(): Promise<AlgorithmStats> {
    return api.get<AlgorithmStats>('/algorithms/stats')
  },

  getById(id: number | string): Promise<AlgorithmItem> {
    return api.get<AlgorithmItem>(`/algorithms/${encodeURIComponent(id)}`)
  },

  listVersions(id: number | string): Promise<AlgorithmVersionItem[]> {
    return api.get<AlgorithmVersionItem[]>(`/algorithms/${encodeURIComponent(id)}/versions`)
  },

  uploadPackage(file: File): Promise<UploadAlgorithmResponse> {
    return uploadFormData<UploadAlgorithmResponse>('/algorithms/upload', file)
  },

  activateVersion(id: number | string, version: string): Promise<void> {
    return api.put<void>(
      `/algorithms/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}/activate`,
      {},
    )
  },

  uninstallVersion(id: number | string, version: string): Promise<void> {
    return api.delete<void>(
      `/algorithms/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}`,
    )
  },
}

export const instanceApi = {
  list(cameraId?: string): Promise<AlgorithmInstanceDto[]> {
    const qs = toQueryString({ cameraId })
    return api.get<AlgorithmInstanceDto[]>(`/tasks/instances${qs}`)
  },

  create(data: CreateAlgorithmInstanceRequest): Promise<AlgorithmInstanceDto> {
    return api.post<AlgorithmInstanceDto>('/tasks/instances', data)
  },

  update(instanceId: string, data: UpdateAlgorithmInstanceRequest): Promise<AlgorithmInstanceDto> {
    return api.put<AlgorithmInstanceDto>(`/tasks/instances/${encodeURIComponent(instanceId)}`, data)
  },

  setEnabled(instanceId: string, enabled: boolean): Promise<void> {
    return api.put<void>(`/tasks/instances/${encodeURIComponent(instanceId)}/enabled`, { enabled })
  },

  delete(instanceId: string): Promise<void> {
    return api.delete<void>(`/tasks/instances/${encodeURIComponent(instanceId)}`)
  },
}

export const gb28181Api = {
  getConfig(): Promise<Gb28181ConfigResponse> {
    return api.get<Gb28181ConfigResponse>('/system/gb28181/config')
  },

  updateConfig(data: UpdateGb28181ConfigRequest): Promise<SysGb28181Config> {
    return api.put<SysGb28181Config>('/system/gb28181/config', data)
  },

  listDevices(): Promise<Gb28181Device[]> {
    return api.get<Gb28181Device[]>('/system/gb28181/devices')
  },

  syncCatalog(deviceId: string): Promise<void> {
    return api.post<void>(`/system/gb28181/devices/${encodeURIComponent(deviceId)}/sync`, {})
  },

  batchImportChannels(data: BatchImportGbChannelsRequest): Promise<BatchImportGbChannelsResponse> {
    return api.post<BatchImportGbChannelsResponse>('/system/gb28181/channels/import', data)
  },

  scanDiscovery(): Promise<DiscoveredDevice[]> {
    return api.get<DiscoveredDevice[]>('/system/discovery/scan')
  },
}
