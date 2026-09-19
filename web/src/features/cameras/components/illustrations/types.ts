export type CameraModelType = 'dome' | 'bullet' | 'ptz'
export type CameraOperationalStatus = 'online' | 'offline' | 'warning'

export const ILLUSTRATION_STATUS_COLORS: Record<CameraOperationalStatus, string> = {
  online: '#10b981',
  warning: '#f59e0b',
  offline: '#94a3b8',
}

export interface CameraIllustrationProps {
  status: CameraOperationalStatus
  aiActive?: boolean
  ariaLabel?: string
  className?: string
  width?: number | string
  height?: number | string
}
