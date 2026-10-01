import type { Camera } from '@/types'

type CameraNameSearchFields = Pick<Camera, 'name'>
type CameraAddressSearchFields = Pick<
  Camera,
  'cameraId' | 'rtspUrl' | 'subRtspUrl' | 'gb28181DeviceId' | 'gb28181ChannelId'
>

function includesNormalizedQuery(
  value: string | null | undefined,
  normalizedQuery: string,
): boolean {
  return value?.toLowerCase().includes(normalizedQuery) ?? false
}

export function normalizeCameraSearchQuery(query: string): string {
  return query.trim().toLowerCase()
}

export function matchesCameraNameQuery(
  camera: CameraNameSearchFields,
  query: string,
  preNormalizedQuery?: string,
): boolean {
  const normalizedQuery = preNormalizedQuery ?? normalizeCameraSearchQuery(query)
  if (!normalizedQuery) return true
  return includesNormalizedQuery(camera.name, normalizedQuery)
}

export function matchesCameraAddressQuery(
  camera: CameraAddressSearchFields,
  query: string,
  preNormalizedQuery?: string,
): boolean {
  const normalizedQuery = preNormalizedQuery ?? normalizeCameraSearchQuery(query)
  if (!normalizedQuery) return true

  return (
    includesNormalizedQuery(camera.cameraId, normalizedQuery) ||
    includesNormalizedQuery(camera.rtspUrl, normalizedQuery) ||
    includesNormalizedQuery(camera.subRtspUrl, normalizedQuery) ||
    includesNormalizedQuery(camera.gb28181DeviceId, normalizedQuery) ||
    includesNormalizedQuery(camera.gb28181ChannelId, normalizedQuery)
  )
}
