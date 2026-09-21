import type { AlgorithmUploadProgress, SandboxStepStatus } from '@/types'

const SANDBOX_STEP_STATUSES: readonly SandboxStepStatus[] = ['running', 'passed', 'failed']

function isSandboxStepStatus(value: unknown): value is SandboxStepStatus {
  return typeof value === 'string' && SANDBOX_STEP_STATUSES.includes(value as SandboxStepStatus)
}

/** Validate the untrusted payload received from the global event WebSocket. */
export function isAlgorithmUploadProgress(value: unknown): value is AlgorithmUploadProgress {
  if (!value || typeof value !== 'object') return false

  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.uploadId === 'string' &&
    candidate.uploadId.length > 0 &&
    Number.isInteger(candidate.step) &&
    Number(candidate.step) >= 1 &&
    Number(candidate.step) <= 6 &&
    isSandboxStepStatus(candidate.status)
  )
}
