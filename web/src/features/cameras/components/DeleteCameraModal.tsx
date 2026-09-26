import React, { useCallback, useState } from 'react'
import { Trash2 } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ConfirmDialog } from '@/components/ui/ConfirmDialog'
import { cameraApi } from '@/lib/api'
import type { Camera } from '@/types'

export interface DeleteCameraModalProps {
  isOpen: boolean
  camera: Camera | null
  onClose: () => void
  onSuccess: (cameraId: string) => void
}

/**
 * 删除摄像头二次确认。
 *
 * 外壳、焦点陷阱与 ESC/Enter 语义由共享 ConfirmDialog 承担；本组件负责
 * 删除请求与级联影响说明。
 *
 * 保留最后一次非空 camera：关闭弹窗时调用方会把 props 置空，而退出动画期间
 * 仍需显示名称。用渲染期状态调整（而非 effect 回写），避免「新 camera 已到但
 * 旧值未清」的中间态被提交一帧。
 */
export function DeleteCameraModal({
  isOpen,
  camera,
  onClose,
  onSuccess,
}: DeleteCameraModalProps): React.ReactElement {
  const { t } = useTranslation(['camera', 'common'])

  const [isDeleting, setIsDeleting] = useState(false)
  const [errorMsg, setErrorMsg] = useState<string | null>(null)
  const [activeCamera, setActiveCamera] = useState<Camera | null>(camera)
  if (camera && camera !== activeCamera) {
    setActiveCamera(camera)
  }

  const currentCamera = camera ?? activeCamera

  const handleDelete = useCallback(async () => {
    if (!camera) return
    setIsDeleting(true)
    setErrorMsg(null)
    try {
      await cameraApi.delete(camera.cameraId)
      onSuccess(camera.cameraId)
      onClose()
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('manage.deleteFailed')
      setErrorMsg(msg)
    } finally {
      setIsDeleting(false)
    }
  }, [camera, onSuccess, onClose, t])

  return (
    <ConfirmDialog
      isOpen={isOpen && Boolean(currentCamera)}
      title={t('manage.deleteTitle')}
      description={
        currentCamera
          ? t('manage.deleteMessage', {
              name: currentCamera.name,
              cameraId: currentCamera.cameraId,
            })
          : undefined
      }
      confirmLabel={isDeleting ? t('manage.deleting') : t('manage.confirmDelete')}
      cancelLabel={t('common:cancel')}
      icon={Trash2}
      isConfirming={isDeleting}
      errorMessage={errorMsg}
      onConfirm={handleDelete}
      onClose={onClose}
      showKeyboardHint
    >
      <p className="border-status-warning-border bg-status-warning-soft text-status-warning rounded-xl border p-3 text-xs leading-relaxed">
        {t('manage.deleteWarning')}
      </p>
    </ConfirmDialog>
  )
}
