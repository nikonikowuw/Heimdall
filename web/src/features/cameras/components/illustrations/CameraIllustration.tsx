import React from 'react'
import type { Camera } from '@/types'
import { DomeCameraIllustration } from './DomeCameraIllustration'
import { BulletCameraIllustration } from './BulletCameraIllustration'
import { PtzCameraIllustration } from './PtzCameraIllustration'
import type { CameraIllustrationProps, CameraModelType, CameraOperationalStatus } from './types'
import { resolveCameraModelType } from './cameraModelType'

export interface UnifiedCameraIllustrationProps extends Omit<CameraIllustrationProps, 'status'> {
  type?: CameraModelType
  status?: CameraOperationalStatus
  camera?: Partial<Camera>
}

export function CameraIllustration({
  type,
  status = 'online',
  aiActive = false,
  ariaLabel,
  camera,
  className = '',
  width = '100%',
  height = '100%',
}: UnifiedCameraIllustrationProps): React.ReactElement {
  const resolvedType = resolveCameraModelType(camera, type)
  const sharedProps = { status, aiActive, ariaLabel, className, width, height }

  switch (resolvedType) {
    case 'bullet':
      return <BulletCameraIllustration {...sharedProps} />
    case 'ptz':
      return <PtzCameraIllustration {...sharedProps} />
    case 'dome':
    default:
      return <DomeCameraIllustration {...sharedProps} />
  }
}

export { DomeCameraIllustration, BulletCameraIllustration, PtzCameraIllustration }
