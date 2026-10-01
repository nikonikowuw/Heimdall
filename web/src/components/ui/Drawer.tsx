import { useId, type ReactElement, type ReactNode } from 'react'
import { CloseIconButton } from '@/components/ui/CloseIconButton'
import { ModalOverlay, type ModalOverlayLayer } from '@/components/ui/ModalOverlay'
import { cn } from '@/lib/utils'

export type DrawerSize = 'small' | 'compact' | 'medium' | 'wide'

export interface DrawerProps {
  isOpen: boolean
  onClose: () => void
  closeLabel: string
  closeTitle?: string
  title: ReactNode
  /** 标题换行或被挤压时的原生悬浮提示；语义与 CloseIconButton 的 title 一致 */
  titleTooltip?: string
  description?: ReactNode
  icon?: ReactNode
  metadata?: ReactNode
  headerActions?: ReactNode
  toolbar?: ReactNode
  footer?: ReactNode
  size?: DrawerSize
  layer?: ModalOverlayLayer
  closeDisabled?: boolean
  priority?: number
  onExitComplete?: () => void
  bodyClassName?: string
  children: ReactNode
}

const SIZE_CLASS: Record<DrawerSize, string> = {
  small: 'modal-surface--small',
  compact: 'modal-surface--compact',
  medium: 'modal-surface--drawer-medium',
  wide: 'modal-surface--drawer-wide',
}

export function Drawer({
  isOpen,
  onClose,
  closeLabel,
  closeTitle,
  title,
  titleTooltip,
  description,
  icon,
  metadata,
  headerActions,
  toolbar,
  footer,
  size = 'medium',
  layer = 'base',
  closeDisabled = false,
  priority = 0,
  onExitComplete,
  bodyClassName,
  children,
}: DrawerProps): ReactElement {
  const titleId = useId()
  const descriptionId = useId()

  // 无障碍名称始终来自标题节点：`aria-labelledby` 会覆盖 `aria-label`，
  // 因此这里不再重复传 ariaLabel（旧版本传了但从未渲染到 DOM）。
  return (
    <ModalOverlay
      isOpen={isOpen}
      onClose={onClose}
      ariaLabelledBy={titleId}
      ariaDescribedBy={description ? descriptionId : undefined}
      variant="drawer"
      surface="solid"
      layer={layer}
      closeDisabled={closeDisabled}
      priority={priority}
      onExitComplete={onExitComplete}
      panelClassName={cn('p-0', SIZE_CLASS[size])}
    >
      <header className="drawer-header">
        <div className="drawer-header__main">
          {icon && <div className="modal-form-icon">{icon}</div>}
          <div className="drawer-header__copy">
            <div className="drawer-header__title-row">
              <h2 id={titleId} title={titleTooltip} className="drawer-header__title">
                {title}
              </h2>
              {metadata}
            </div>
            {description && (
              <div id={descriptionId} className="drawer-header__description">
                {description}
              </div>
            )}
          </div>
        </div>
        <div className="drawer-header__actions">
          {headerActions}
          <CloseIconButton
            onClick={onClose}
            label={closeLabel}
            title={closeTitle}
            ariaDisabled={closeDisabled}
          />
        </div>
      </header>

      {toolbar && <div className="drawer-toolbar">{toolbar}</div>}

      <div className={cn('drawer-body', bodyClassName)}>{children}</div>

      {footer && <div className="drawer-footer">{footer}</div>}
    </ModalOverlay>
  )
}
