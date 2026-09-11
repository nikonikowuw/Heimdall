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

export const WS_TOPICS = {
  CAMERA_PROBE_UPDATED: 'camera.probe_updated',
  ALARM_TRIGGERED: 'alarm.triggered',
  ALARM_STATUS_CHANGED: 'alarm.status_changed',
  CAMERA_TRACKS: 'camera.tracks',
} as const

export type WsTopic = (typeof WS_TOPICS)[keyof typeof WS_TOPICS]

export type AlarmStatus = 'unprocessed' | 'processed'
export type AlarmSeverity = 'warning' | 'critical'

export type ProbeStatus = 'never' | 'healthy' | 'success' | 'degraded' | 'reconnecting' | 'failed'

export type StreamMode = 'auto' | 'main' | 'sub'

export interface Camera {
  id: number
  cameraId: string
  name: string
  protocol: string
  rtspUrl: string
  subRtspUrl: string
  streamMode: StreamMode
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
  streamMode?: StreamMode
  remark?: string
  transportPolicy?: 'auto' | 'tcp' | 'udp'
  gb28181DeviceId?: string
  gb28181ChannelId?: string
}

export interface UpdateCameraRequest {
  name?: string
  rtspUrl?: string
  subRtspUrl?: string
  streamMode?: StreamMode
  remark?: string
  transportPolicy?: 'auto' | 'tcp' | 'udp'
  gb28181DeviceId?: string
  gb28181ChannelId?: string
}

export interface SubStreamCandidate {
  brand: string
  subUrl: string
  description: string
}

export interface TrackedBBox {
  trackId: number
  label: string
  confidence: number
  bbox: [number, number, number, number] // [x1, y1, x2, y2] 归一化坐标 0.0 ~ 1.0
  trajectory?: [number, number][]
}

export interface CameraTracksPayload {
  cameraId: string
  timestamp: number
  tracks: TrackedBBox[]
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
  occurredAt: number
  targetLabel: string
  confidence: number
  trackId: number
  bboxJson: string
  imageId: string
  imageRelPath: string
  cropImageId?: string
  cropImageRelPath?: string
  ruleType?: string
  severity?: AlarmSeverity
  status: AlarmStatus
  handledAt?: number | null
  createdAt: number
}

export interface CaptureRecord {
  id: number
  captureId: string
  cameraId: string
  trackId: number
  targetLabel: string
  confidence: number
  qualityScore: number
  bboxJson: string
  imageId: string
  imageRelPath: string
  cropImageId: string
  cropImageRelPath: string
  capturedAt: number
  createdAt: number
}

export interface RecognitionRecord {
  id: number
  recognitionId: string
  cameraId: string
  galleryId: string
  subjectId: string
  subjectName: string
  similarity: number
  fieldCropPath: string
  registeredPhotoPath: string
  recognizedAt: number
  createdAt: number
}

export interface PersonnelItem {
  id: number
  subjectId: string
  name: string
  idCard: string
  remark: string
  primaryPhotoPath: string
  faceCount: number
  createdAt: number
  updatedAt: number
}

export interface GalleryFace {
  id: number
  faceId: string
  subjectId: string
  photoRelPath: string
  alignedRelPath: string
  qualityScore: number
  detectionScore: number
  isPrimary: boolean
  createdAt: number
}

export interface PersonnelDetail {
  id: number
  subjectId: string
  name: string
  idCard: string
  remark: string
  primaryPhotoPath: string
  faces: GalleryFace[]
  createdAt: number
  updatedAt: number
}

export interface PersonnelStats {
  totalPersonnel: number
  totalFaces: number
  algoReady: boolean
}

export interface AlgoManifest {
  algorithmId: string
  name: string
  version: string
  author: string
  description: string
  category: string
  algorithmType?: string
  supportedPlatforms: string[]
  classes: string[]
  license?: string
  configSchema?: Record<string, unknown>
}

export interface SandboxCheckResult {
  passed: boolean
  stepsTotal: number
  stepsPassed: number
  steps: string[]
  errorMessage?: string
  manifest?: AlgoManifest
}

export interface MotionGateConfig {
  enabled: boolean
  threshold?: number
  contourArea?: number
  keepaliveIntervalMs?: number
}

export interface TaskAlgorithmInstanceDto {
  instanceId?: string
  algorithmId: string
  analysisFps?: number
  algoParams?: Record<string, unknown>
  enabled?: boolean
  actualStatus?: number
  statusMessage?: string
  createdAt?: number
  updatedAt?: number
}

export interface TaskAlgorithmInstanceSummaryDto {
  instanceId: string
  algorithmId: string
  analysisFps: number
  enabled: boolean
  actualStatus: number
  statusMessage: string
}

export interface TaskSummaryDto {
  id: number
  cameraId: string
  name: string
  desiredEnabled: boolean
  actualStatus: number
  statusMessage?: string
  algorithmId?: string
  analysisFps?: number
  algoParams?: Record<string, unknown>
  algorithmInstanceCount?: number
  algorithmInstances?: TaskAlgorithmInstanceSummaryDto[]
  rulesCount: number
  motionGateEnabled: boolean
  rules: DetectionRule[]
  motionGate?: MotionGateConfig
  createdAt: number
  updatedAt: number
}

export interface TaskConfigDto {
  cameraId: string
  name: string
  desiredEnabled: boolean
  streamMode?: StreamMode
  algorithmId?: string
  analysisFps?: number
  algoParams?: Record<string, unknown>
  actualStatus?: number
  statusMessage?: string
  algorithmInstances?: TaskAlgorithmInstanceDto[]
  rules: DetectionRule[]
  motionGate?: MotionGateConfig
}

export interface AlgorithmVersionItem {
  id: number
  algorithmId: string
  version: string
  platformId: string
  minAdapterVersion: string
  packageRoot: string
  fpsTiers: { fps: number; units: number }[]
  configSchema: Record<string, unknown>
  manifestRaw: Record<string, unknown>
  packageSizeBytes: number
  isActive: boolean
  isBuiltin: boolean
  createdAt: number
  updatedAt: number
}

export interface AlgorithmItem {
  id: number
  algorithmId: string
  name: string
  algorithmType: string
  alarmTypeId: string
  activeVersion: string
  description: string
  isBuiltin: boolean
  createdAt: number
  updatedAt: number
  versions: AlgorithmVersionItem[]
}

export interface AlgorithmStats {
  totalAlgorithms: number
  totalActiveVersions: number
  builtinAlgorithms: number
  customAlgorithms: number
}

export interface PaginatedAlgorithms {
  items: AlgorithmItem[]
  total: number
}

export interface UploadAlgorithmResponse {
  passed: boolean
  stepsTotal: number
  stepsPassed: number
  steps: string[]
  errorMessage?: string
  version?: {
    algorithmId: string
    version: string
    platformId: string
    packageRoot: string
    isActive: boolean
  }
  manifest?: Record<string, unknown>
}

export interface AlgorithmInstanceDto {
  id: number
  instanceId: string
  cameraId: string
  algorithmId: string
  analysisFps: number
  params: Record<string, unknown>
  rules: DetectionRule[]
  motionGate: MotionGateConfig
  enabled: boolean
  actualStatus: number
  statusMessage: string
  createdAt: number
  updatedAt: number
}

export interface CreateAlgorithmInstanceRequest {
  cameraId: string
  algorithmId: string
  analysisFps?: number
  params?: Record<string, unknown>
  rules?: DetectionRule[]
  motionGate?: MotionGateConfig
  enabled?: boolean
}

export interface UpdateAlgorithmInstanceRequest {
  analysisFps?: number
  params?: Record<string, unknown>
  rules?: DetectionRule[]
  motionGate?: MotionGateConfig
  enabled?: boolean
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
  createdAt: number
}
