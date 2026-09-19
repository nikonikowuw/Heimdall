import type { Camera } from '@/types'
import type { CameraModelType } from './types'

export const CAMERA_MODEL_CHANGE_EVENT = 'heimdall:camera-model-change'

export interface CameraModelChangeEventDetail {
  cameraId: string
  type: CameraModelType
}

export function getSavedCameraModelType(cameraId: string): CameraModelType | null {
  if (typeof window === 'undefined') return null
  try {
    const saved = localStorage.getItem(`heimdall_cam_model_${cameraId}`)
    if (saved === 'dome' || saved === 'bullet' || saved === 'ptz') {
      return saved
    }
  } catch {
    // 忽略在无权限或 SSR 环境下的 Storage 异常
  }
  return null
}

export function saveCameraModelType(cameraId: string, type: CameraModelType): void {
  if (typeof window === 'undefined') return
  try {
    localStorage.setItem(`heimdall_cam_model_${cameraId}`, type)
  } catch {
    // 忽略在无权限或配额超限下的 Storage 异常
  }
}

/** 获取摄像头形态对应的本地化名称标签 */
export function getCameraTypeLabel(
  type: CameraModelType,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  switch (type) {
    case 'dome':
      return t('tile.dome', { defaultValue: '半球摄像机' })
    case 'ptz':
      return t('tile.ptz', { defaultValue: '云台球机' })
    case 'bullet':
    default:
      return t('tile.bullet', { defaultValue: '筒形枪机' })
  }
}

/**
 * 根据摄像头对象的元数据识别其物理形态（Dome / Bullet / PTZ）
 * 默认以安防领域最通用的标准固定枪机 (Bullet) 作为基准形态，仅在明确包含云台/球机或半球特征时切换。
 */
export function resolveCameraModelType(
  camera?: Partial<Camera> | null,
  explicitType?: CameraModelType,
): CameraModelType {
  if (explicitType) {
    return explicitType
  }

  if (!camera) {
    return 'bullet'
  }

  const searchTarget = [
    camera.name,
    camera.remark,
    camera.rtspUrl,
    camera.subRtspUrl,
    camera.cameraId,
  ]
    .filter(Boolean)
    .join(' ')
    .toLowerCase()

  // 1. 明确包含球机/云台关键字时才识别为 PTZ
  if (
    searchTarget.includes('ptz') ||
    searchTarget.includes('球机') ||
    searchTarget.includes('云台') ||
    searchTarget.includes('speed-dome') ||
    searchTarget.includes('speed_dome') ||
    searchTarget.includes('speeddome')
  ) {
    return 'ptz'
  }

  // 2. 包含半球/室内/吸顶关键字时识别为 Dome
  if (
    searchTarget.includes('dome') ||
    searchTarget.includes('半球') ||
    searchTarget.includes('吸顶') ||
    searchTarget.includes('电梯')
  ) {
    return 'dome'
  }

  // 3. 包含枪机/筒机关键字
  if (
    searchTarget.includes('bullet') ||
    searchTarget.includes('枪机') ||
    searchTarget.includes('筒机')
  ) {
    return 'bullet'
  }

  // 4. 默认安防标准网络摄像头一律呈现通用枪机形态，绝不胡乱分配为云台球机
  return 'bullet'
}
