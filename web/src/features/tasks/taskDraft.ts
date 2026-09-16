import type {
  AlgorithmItem,
  AlgorithmVersionItem,
  Camera,
  TaskAlgorithmInstanceDto,
  TaskConfigDto,
} from '@/types'
import { buildSchemaDefaultParams } from './algoMetadata'

/**
 * 任务写入载荷的纯逻辑层。
 *
 * 后端 `PUT /api/v1/tasks/{cameraId}` 按 cameraId 幂等，且 `rules` / `motionGate` /
 * `algorithmInstances` 三个字段是全量覆盖写（见 `crates/db/src/repository/task.rs`
 * `save_task_with_instances_txn`）。因此载荷组装必须精确表达「本次意图」，
 * 未涉及的字段一律省略或原样回传，避免以写入之名清空既有配置。
 *
 * 唯一的例外是「布防开关」：它由状态动词 `PUT /api/v1/tasks/{cameraId}/enabled`
 * 单独承担，不需要在这里构造任何配置载荷。
 */

/** 快速创建时的推理算力起步档；与工作台首次挂载算法保持一致（LiveRulesStudio.handleToggleAlgo） */
export const DEFAULT_ANALYSIS_FPS = 10

export interface CameraBuckets {
  /** 尚未建立任务的通道，可作为快速创建的候选 */
  creatable: Camera[]
  /** 已建立任务的通道数：其配置只能在布防工作台内修改 */
  configuredCount: number
}

/**
 * 按「一设备一任务」拆分通道。
 *
 * `analysis_tasks.camera_id` 为 UNIQUE，一个通道只会有一条任务；已配置通道若进入创建候选，
 * 创建即等同覆盖既有防区与算法栈，因此必须从候选中剔除。
 */
export function splitCamerasByTask(
  cameras: Camera[],
  taskCameraIds: ReadonlySet<string>,
): CameraBuckets {
  const creatable: Camera[] = []
  let configuredCount = 0

  for (const camera of cameras) {
    if (taskCameraIds.has(camera.cameraId)) {
      configuredCount += 1
    } else {
      creatable.push(camera)
    }
  }

  return { creatable, configuredCount }
}

/** 字节序比较 algorithmId，与后端 `str::cmp` 同口径，避免推荐结果前后端漂移 */
function compareAlgorithmId(left: AlgorithmItem, right: AlgorithmItem): number {
  if (left.algorithmId === right.algorithmId) return 0
  return left.algorithmId < right.algorithmId ? -1 : 1
}

/**
 * 推荐初始算法，与后端 `task_service::resolve_algorithm_id` 同口径：
 * 优先 `general_detection`，其次按 algorithmId 字典序取首个。
 * 仅在已安装版本（`activeVersion` 非空）中挑选；一个都没有时返回空串，由调用方阻止提交。
 */
export function pickRecommendedAlgorithmId(items: AlgorithmItem[]): string {
  const installed = items.filter((item) => item.activeVersion.trim() !== '')
  const preferred = installed.find((item) => item.algorithmId === 'general_detection')

  if (preferred) return preferred.algorithmId
  return [...installed].sort(compareAlgorithmId)[0]?.algorithmId ?? ''
}

/** 取算法当前生效版本：isActive 标记 → activeVersion 匹配 → 首个版本 */
export function pickActiveVersion(algo?: AlgorithmItem | null): AlgorithmVersionItem | undefined {
  if (!algo) return undefined
  return (
    algo.versions.find((version) => version.isActive) ??
    algo.versions.find((version) => version.version === algo.activeVersion) ??
    algo.versions[0]
  )
}

export interface QuickCreateInput {
  cameraId: string
  name: string
  algorithmId: string
  algorithmVersion?: AlgorithmVersionItem | null
  desiredEnabled: boolean
}

/**
 * 构造快速创建载荷：只表达「通道 + 名称 + 初始算法 + 是否布防」。
 *
 * - 省略 `streamMode`：不在创建阶段改写摄像头级配置，交由工作台或设备管理决定；
 * - 省略 `motionGate`：使用服务端默认门控，而不是前端硬编码常量；
 * - `rules` 恒为 `[]`：新任务本无防区，防区一律在工作台的动态子码流画布上绘制；
 * - `algoParams` 只写算法 schema 声明的键，不注入前端臆造的键名；
 * - `configRevision: 0`：断言「选择该通道时它还没有任务」。若别的会话抢先建了任务，
 *   服务端会以版本冲突拒绝，而不是把对方的实例与防区静默覆盖掉。
 */
export function buildQuickCreatePayload(input: QuickCreateInput): TaskConfigDto {
  const instance: TaskAlgorithmInstanceDto = {
    algorithmId: input.algorithmId,
    analysisFps: DEFAULT_ANALYSIS_FPS,
    algoParams: buildSchemaDefaultParams(input.algorithmVersion),
    enabled: input.desiredEnabled,
  }

  return {
    cameraId: input.cameraId,
    name: input.name.trim(),
    desiredEnabled: input.desiredEnabled,
    rules: [],
    algorithmInstances: [instance],
    configRevision: 0,
  }
}
