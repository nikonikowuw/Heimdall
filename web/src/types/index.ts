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
  CAMERA_TELEMETRY: 'camera.telemetry',
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

export interface SysGb28181Config {
  sipId: string
  sipDomain: string
  sipPort: number
  sipPassword: string
  rtpPortRangeStart: number
  rtpPortRangeEnd: number
  autoCatalogSync: boolean
  heartbeatTimeoutSec: number
  updatedAtMs: number
}

export interface UpdateGb28181ConfigRequest {
  sipId?: string
  sipDomain?: string
  sipPort?: number
  sipPassword?: string
  rtpPortRangeStart?: number
  rtpPortRangeEnd?: number
  autoCatalogSync?: boolean
  heartbeatTimeoutSec?: number
}

export interface Gb28181ServerHealth {
  running: boolean
  sipPort: number
  transport: string
  onlineDevicesCount: number
  totalDevicesCount: number
  activeStreamsCount: number
}

export interface Gb28181ConfigResponse {
  config: SysGb28181Config
  health: Gb28181ServerHealth
}

export interface Gb28181Channel {
  deviceId: string
  channelId: string
  name: string
  manufacturer: string
  model: string
  status: string
  parentId: string
  subStreamSupported: boolean
  lastSeenMs: number
  isImported: boolean
  cameraId?: string | null
}

export interface Gb28181Device {
  deviceId: string
  name: string
  ipAddr: string
  sipPort: number
  transport: string
  status: string
  channelCount: number
  lastKeepaliveMs: number
  createdAtMs: number
  updatedAtMs: number
  channels: Gb28181Channel[]
}

export interface ImportGbChannelItem {
  deviceId: string
  channelId: string
  name?: string
  streamMode?: string
}

export interface BatchImportGbChannelsRequest {
  channels: ImportGbChannelItem[]
}

export interface BatchImportGbChannelsResponse {
  importedCount: number
  cameraIds: string[]
}

export interface DiscoveredDevice {
  ip: string
  port: number
  name: string
  manufacturer: string
  model: string
  protocol: string
  xaddrs?: string | null
  rtspUrl?: string | null
}

export interface FaceTrack {
  bbox: [number, number, number, number] // [x1, y1, x2, y2] 归一化坐标 0.0 ~ 1.0
  confidence: number
  qualityScore?: number
}

export interface TrackedBBox {
  trackId: number
  label: string
  confidence: number
  qualityScore?: number
  bbox: [number, number, number, number] // [x1, y1, x2, y2] 归一化坐标 0.0 ~ 1.0
  face?: FaceTrack // 挂载的人脸详情
  trajectory?: [number, number][]
}

export interface CameraTracksPayload {
  cameraId: string
  timestamp: number
  tracks: TrackedBBox[]
}

export interface CameraTelemetry {
  cameraId: string
  timestamp: number
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
  /** 证据图产生路径；历史记录未标注时为 null */
  imageSource?: EvidenceImageSource | null
  /** 证据图所属码流；历史记录未标注时为 null */
  imageStream?: EvidenceImageStream | null
  /** 证据帧的可比 PTS（检测轴）；不可比或未知时为 null */
  imagePtsMs?: number | null
  /** 匹配所用融合模板的参与帧数（仅新版算法包上报） */
  fusedCount?: number | null
  /** 匹配所用融合模板的质量加权均值 */
  templateQuality?: number | null
  capturedAt: number
  createdAt: number
}

export type RecognitionStatus = 'confirmed' | 'pending_review' | 'rejected'

/** 证据图产生路径：结算峰值候选帧 / 靶向快拍帧 */
export type EvidenceImageSource = 'peak_candidate' | 'targeted'

/** 证据图所属码流：主码流高分辨率帧 / 子码流帧 */
export type EvidenceImageStream = 'main' | 'sub'

export interface FaceCandidateItem {
  rank: number
  subjectId: string
  subjectName: string
  similarity: number
  faceId: string
  photoRelPath?: string
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
  fieldImagePath: string | null
  fieldBboxJson: string | null
  registeredPhotoPath: string
  /** 证据图产生路径；历史记录未标注时为 null */
  imageSource?: EvidenceImageSource | null
  /** 证据图所属码流；历史记录未标注时为 null */
  imageStream?: EvidenceImageStream | null
  /** 证据帧的可比 PTS（检测轴）；不可比或未知时为 null */
  imagePtsMs?: number | null
  /** 1:N 比对所用融合模板的参与帧数（仅新版算法包上报） */
  fusedCount?: number | null
  /** 1:N 比对所用融合模板的质量加权均值 */
  templateQuality?: number | null
  status: RecognitionStatus
  candidates?: FaceCandidateItem[]
  reviewerId?: string | null
  reviewedAt?: number | null
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

export interface ReextractFaceFailureDetail {
  faceId: string
  subjectId: string
  reason: string
}

export type ReextractTaskStatus = 'idle' | 'running' | 'completed' | 'failed'

export interface ReextractProgress {
  status: ReextractTaskStatus
  total: number
  processed: number
  succeeded: number
  failed: number
  currentFaceId?: string | null
  startedAt?: number | null
  finishedAt?: number | null
  failures: ReextractFaceFailureDetail[]
  errorMessage?: string | null
}

export interface ReextractFaceFeaturesReport {
  total: number
  succeeded: number
  failed: number
  failures: ReextractFaceFailureDetail[]
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

/**
 * 算法实例期望配置的运行时收敛状态。
 *
 * 与实例健康状态正交：`applied` 表示期望 revision 已在目标 Worker 生效，
 * `pending` 表示已持久化但仍在排队 / 创建 Worker / 等待帧边界，
 * `failed` 表示期望配置已持久化但目标 Worker 未能使用它（保留旧配置继续运行）。
 */
export type InstanceApplyState = 'applied' | 'pending' | 'failed'

export interface MotionGateConfig {
  enabled: boolean
  threshold?: number
  contourArea?: number
  keepaliveIntervalMs?: number
  motionHoldFrames?: number
}

export interface TaskAlgorithmInstanceDto {
  instanceId?: string
  algorithmId: string
  analysisFps?: number
  algoParams?: Record<string, unknown>
  enabled?: boolean
  actualStatus?: number
  /** 期望配置版本号；每次期望配置提交递增 */
  desiredRevision?: number
  /** 已在目标 Worker 上生效的配置版本号 */
  appliedRevision?: number
  /** 期望配置的运行时收敛状态；未生效时不得显示为应用成功 */
  applyState?: InstanceApplyState
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
  applyState: InstanceApplyState
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
  /** 任务配置版本号：整体下发时必须原样回传给服务端做乐观并发校验 */
  configRevision: number
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
  /**
   * 任务配置版本号。
   *
   * 读取时由服务端返回；保存时必须回传读取到的值，服务端不匹配即拒绝写入（409 / 40903），
   * 避免两个编辑会话基于同一旧快照互相覆盖。省略表示不做版本校验。
   */
  configRevision?: number
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
