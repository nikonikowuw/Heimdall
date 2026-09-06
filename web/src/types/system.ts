// ─── 系统概览（只读） ───

export interface SystemOverview {
  softwareVersion: string
  deviceModel: string
  osInfo: string
  kernelVersion: string
  uptimeSeconds: number
  cpuUsagePercent: number
  memoryUsagePercent: number
  memoryUsedMb: number
  memoryTotalMb: number
  npuUsagePercent: number | null
  diskUsagePercent: number
  diskUsedGb: number
  diskTotalGb: number
  activeCameras: number
  totalCameras: number
  activeTasks: number
  todayAlarms: number
  todayCaptures: number
}

// ─── 存储配置 ───

export interface StorageConfig {
  alarmRetentionDays: number
  alarmQuotaMb: number
  recognitionRetentionDays: number
  recognitionQuotaMb: number
  captureRetentionDays: number
  captureQuotaMb: number
  overwriteMode: 'overwrite' | 'stop'
  autoCleanupEnabled: boolean
  minFreeRatio: number
  targetFreeRatio: number
  emergencyFreeRatio: number
  criticalFreeRatio: number
  batchDeleteSize: number
}

export interface StorageStatus {
  totalGb: number
  usedGb: number
  availableGb: number
  usagePercent: number
  healthLevel: 'normal' | 'evicting' | 'emergency' | 'critical'
  alarmCount: number
  alarmSizeMb: number
  recognitionCount: number
  recognitionSizeMb: number
  captureCount: number
  captureSizeMb: number
}

export interface EvictionReport {
  deletedCount: number
  freedMb: number
  durationMs: number
}

// ─── 对时服务 ───

export interface TimeStatus {
  systemTime: number
  timezone: string
  timezoneOffset: number
  ntpSynced: boolean
  ntpService: string
  ntpServer: string | null
  offsetMs: number | null
}

export interface TimeConfig {
  ntpEnabled: boolean
  ntpServer: string
  timezone: string
}

export interface ForceSyncResponse {
  synced: boolean
}

export interface SetTimeResponse {
  applied: boolean
  previousTime: number
  newTime: number
}

// ─── 网络配置 ───

export interface NetworkInterface {
  name: string
  type: 'ethernet' | 'wifi' | 'loopback' | 'virtual'
  state: 'up' | 'down' | 'unknown'
  mac: string
  manager: 'networkmanager' | 'systemd-networkd' | 'netplan' | 'ifupdown' | 'connman' | 'unmanaged'
  ipv4: IpConfig | null
  capabilities: InterfaceCapabilities
}

export interface IpConfig {
  method: 'dhcp' | 'static' | 'none'
  address: string | null
  prefix: number | null
  gateway: string | null
  dns: string[]
}

export interface InterfaceCapabilities {
  canModifyIp: boolean
  canSetDhcp: boolean
  canSetStatic: boolean
  isManagementInterface: boolean
  reason: string | null
}

export interface NetworkInterfacesResponse {
  interfaces: NetworkInterface[]
  pendingOperation: NetworkChangeOperation | null
}

export interface NetworkUpdateResult {
  applied: boolean
  operation: NetworkChangeOperation | null
}

export interface NetworkChangeOperation {
  id: string
  status: OperationStatus
  interfaceName: string
  oldConfig: IpConfig
  newConfig: IpConfig
  createdAt: number
  confirmDeadlineMs: number
  newAccessUrl: string | null
}

export type OperationStatus = 'pending_confirm' | 'confirmed' | 'restoring' | 'restored' | 'failed'

export interface OperationConfirmResult {
  status: OperationStatus
  confirmedAt: number | null
}
