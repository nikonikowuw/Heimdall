import { renderToString } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { DeleteTaskModal } from './DeleteTaskModal'

vi.mock('react-i18next', async (importOriginal) => {
  const actual = await importOriginal<typeof import('react-i18next')>()
  return {
    ...actual,
    useTranslation: () => ({
      t: (key: string, options?: { name?: string }) =>
        key === 'deleteTaskMessage'
          ? `Are you sure you want to delete task "${options?.name}"?`
          : key,
      i18n: { language: 'en' },
    }),
  }
})

describe('DeleteTaskModal', () => {
  it('shows the camera ID used by the delete request even when a task name exists', () => {
    const html = renderToString(
      <DeleteTaskModal
        isOpen
        cameraId="camera-42"
        taskName="Warehouse entrance"
        onClose={() => {}}
        onSuccess={() => {}}
      />,
    )

    expect(html).toContain('Warehouse entrance')
    expect(html).toContain('camera-42')
  })
})
