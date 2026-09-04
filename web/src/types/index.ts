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

export type ProbeStatus = 'never' | 'success' | 'failed'

export interface Camera {
  id: number
  cameraId: string
  name: string
  protocol: string
  rtspUrl: string
  subRtspUrl: string
  remark: string
  lastProbeStatus: ProbeStatus
  lastProbeAt: string | null
  lastCodec: string
  lastWidth: number
  lastHeight: number
  lastFps: number
  createdAt: string
  updatedAt: string
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
