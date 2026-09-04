export interface ApiResponse<T> {
  code: number
  message: string
  data: T
  timestamp: number
}

export interface InitStatusResponse {
  initialized: boolean
}

export interface InitializeRequest {
  username: string
  password: string
}

export interface LoginRequest {
  username: string
  password: string
}

export interface LoginResponse {
  accessToken: string
  username: string
  expiresAt: number
}

export interface ChangePasswordRequest {
  oldPassword: string
  newPassword: string
}

export interface AdminUserDto {
  username: string
  createdAt: number
  updatedAt: number
}

export type ProbeStatus = 'never' | 'healthy' | 'success' | 'degraded' | 'reconnecting' | 'failed'

export interface Camera {
  id: number
  cameraId: string
  name: string
  protocol: string
  rtspUrl: string
  subRtspUrl: string
  remark: string
  lastProbeStatus: ProbeStatus
  lastProbeAt?: number | null
  lastProbeErrorCode?: string
  lastSuccessAt?: number | null
  lastCodec: string
  lastWidth: number
  lastHeight: number
  lastFps: number
  gb28181DeviceId?: string | null
  gb28181ChannelId?: string | null
  createdAt: number
  updatedAt: number
}

export interface CreateCameraRequest {
  name: string
  protocol?: 'rtsp' | 'gb28181'
  rtspUrl: string
  subRtspUrl?: string
  remark?: string
  transportPolicy?: 'auto' | 'tcp' | 'udp'
  gb28181DeviceId?: string
  gb28181ChannelId?: string
}

export interface UpdateCameraRequest {
  name?: string
  rtspUrl?: string
  subRtspUrl?: string
  remark?: string
  transportPolicy?: 'auto' | 'tcp' | 'udp'
  gb28181DeviceId?: string
  gb28181ChannelId?: string
}

export interface TrackedBBox {
  trackId: number
  label: string
  confidence: number
  bbox: [number, number, number, number] // [x1, y1, x2, y2] 归一化坐标 0.0 ~ 1.0
  trajectory?: [number, number][]
}

export interface CameraTelemetry {
  cameraId: string
  activeTracks: number
  personCount: number
  carCount: number
  motionScore: number
  isMotionGated: boolean
}

export type DetectionRuleRole = 'roi' | 'mask' | 'line'
export type DetectionLineDirection = 'both' | 'a_to_b' | 'b_to_a'

export interface DetectionPoint {
  x: number
  y: number
}

export interface DetectionRule {
  role: DetectionRuleRole
  lineDirection?: DetectionLineDirection
  points: DetectionPoint[]
}

export interface AnalysisTask {
  id: number
  cameraId: string
  name: string
  desiredEnabled: boolean
  actualStatus: number
  rulesJson: string
  motionGateJson: string
  createdAt: string
  updatedAt: string
}

export interface AlarmRecord {
  id: number
  eventId: string
  cameraId: string
  alarmTypeId: string
  occurredAt: string
  targetLabel: string
  confidence: number
  trackId: number
  bboxJson: string
  imageId: string
  imageRelPath: string
  createdAt: string
}

export interface OperationLog {
  id: number
  username: string
  module: string
  action: string
  method: string
  path: string
  query: string
  body: string
  statusCode: number
  durationMs: number
  ip: string
  userAgent: string
  createdAt: string
}
