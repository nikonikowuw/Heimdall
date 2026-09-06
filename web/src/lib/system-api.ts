import { request } from './api'
import type {
  SystemOverview,
  NetworkInterfacesResponse,
  NetworkInterface,
  NetworkUpdateResult,
  NetworkChangeOperation,
  OperationConfirmResult,
  StorageConfig,
  StorageStatus,
  EvictionReport,
  TimeConfig,
  TimeStatus,
  IpConfig,
  ForceSyncResponse,
  SetTimeResponse,
} from '../types/system'

async function get<T>(endpoint: string, signal?: AbortSignal): Promise<T> {
  return request<T>(endpoint, { signal })
}

async function put<T>(endpoint: string, body: unknown): Promise<T> {
  return request<T>(endpoint, {
    method: 'PUT',
    body: JSON.stringify(body),
  })
}

async function post<T>(endpoint: string, body?: unknown): Promise<T> {
  return request<T>(endpoint, {
    method: 'POST',
    body: body !== undefined ? JSON.stringify(body) : undefined,
  })
}

// ─── 系统概览 ───

export const systemApi = {
  getOverview: (signal?: AbortSignal) => get<SystemOverview>('/system/overview', signal),

  // ─── 网络配置 ───

  getNetworkInterfaces: (signal?: AbortSignal) =>
    get<NetworkInterfacesResponse>('/system/network/interfaces', signal),

  getNetworkInterface: (name: string, signal?: AbortSignal) =>
    get<NetworkInterface>(`/system/network/interfaces/${encodeURIComponent(name)}`, signal),

  updateNetworkInterface: (name: string, config: Partial<IpConfig>) =>
    put<NetworkUpdateResult>(`/system/network/interfaces/${encodeURIComponent(name)}`, config),

  getPendingNetworkChange: (signal?: AbortSignal) =>
    get<NetworkChangeOperation | null>('/system/network/changes/pending', signal),

  confirmNetworkChange: (id: string) =>
    post<OperationConfirmResult>(`/system/network/changes/${encodeURIComponent(id)}/confirm`),

  cancelNetworkChange: (id: string) =>
    post<OperationConfirmResult>(`/system/network/changes/${encodeURIComponent(id)}/cancel`),

  // ─── 存储与保留策略 ───

  getStorageStatus: (signal?: AbortSignal) => get<StorageStatus>('/system/storage/status', signal),

  getStorageConfig: (signal?: AbortSignal) => get<StorageConfig>('/system/storage/config', signal),

  updateStorageConfig: (config: StorageConfig) =>
    put<StorageConfig>('/system/storage/config', config),

  triggerCleanup: () => post<EvictionReport>('/system/storage/cleanup'),

  // ─── 对时服务 ───

  getTimeStatus: (signal?: AbortSignal) => get<TimeStatus>('/system/time/status', signal),

  getTimeConfig: (signal?: AbortSignal) => get<TimeConfig>('/system/time/config', signal),

  updateTimeConfig: (config: TimeConfig) => put<TimeConfig>('/system/time/config', config),

  forceTimeSync: () => post<ForceSyncResponse>('/system/time/sync'),

  setSystemTime: (time: string) => post<SetTimeResponse>('/system/time/set', { time }),
}
