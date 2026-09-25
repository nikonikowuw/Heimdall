import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { PersonnelItem } from '@/types'
import { BatchDeleteModal } from './components/BatchDeleteModal'
import { DeleteConfirmModal } from './components/DeleteConfirmModal'
import { PersonnelBatchBar } from './components/PersonnelBatchBar'
import { PersonnelCard } from './components/PersonnelCard'
import { PersonnelImportModal } from './components/PersonnelImportModal'
import { PersonnelModal } from './components/PersonnelModal'
import { isSupportedArchiveName, isTerminalImportStatus } from './components/personnelImport'
import { PersonnelTable } from './components/PersonnelTable'
import { PersonnelToast } from './components/PersonnelToast'
import { ReextractModal } from './components/ReextractModal'
import { PersonnelPage } from './PersonnelPage'

const person: PersonnelItem = {
  id: 1,
  subjectId: 'S-001',
  name: '张三',
  idCard: '110101199001010011',
  remark: '安保部',
  primaryPhotoPath: 'faces/a.jpg',
  faceCount: 3,
  createdAt: 1_700_000_000_000,
  updatedAt: 1_700_000_000_000,
}

describe('PersonnelPage composition', () => {
  it('exposes the quick-search field and the empty state entry point before data arrives', () => {
    const html = renderToString(<PersonnelPage />)

    expect(html).toContain('data-search-input="true"')
    // 未加载完成前不把统计渲染成 0，避免被误读为空底库
    expect(html).toContain('—')
  })
})

describe('Personnel floating layers', () => {
  it('marks the register dialog as a labelled modal', () => {
    const html = renderToString(
      <PersonnelModal isOpen onClose={() => {}} onSuccess={() => {}} editTarget={null} />,
    )

    expect(html).toContain('role="dialog"')
    expect(html).toContain('aria-modal="true"')
    expect(html).toContain('aria-labelledby=')
    expect(html).toContain('aria-describedby=')
  })

  it('uses semantic theme tokens for register modal states', () => {
    const html = renderToString(
      <PersonnelModal isOpen onClose={() => {}} onSuccess={() => {}} editTarget={null} />,
    )

    expect(html).toContain('modal-form-header')
    expect(html).toContain('modal-form-field')
    expect(html).toContain('modal-form-button--primary')
    expect(html).toContain('modal-surface--form')
    expect(html).not.toContain('modal-surface--glass')
    expect(html).toContain('focus-visible:ring-[var(--ring)]')
    expect(html).toContain('bg-[var(--bg-surface-solid)]')
    expect(html).not.toMatch(/(?:emerald|rose|amber)-\d+/)
  })

  it('labels the destructive confirmation with the target identity', () => {
    const html = renderToString(
      <DeleteConfirmModal
        isOpen
        target={person}
        isDeleting={false}
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    )

    expect(html).toContain('role="alertdialog"')
    expect(html).toContain('张三')
    expect(html).toContain('S-001')
  })

  it('reports re-extract progress through an accessible progressbar', () => {
    const html = renderToString(
      <ReextractModal
        isOpen
        isGlobal
        progress={{
          status: 'running',
          total: 10,
          processed: 4,
          succeeded: 3,
          failed: 1,
          failures: [],
        }}
        isStarting={false}
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    )

    expect(html).toContain('role="progressbar"')
    expect(html).toContain('aria-valuenow="40"')
  })

  it('announces feedback through a status region instead of a silent update', () => {
    const html = renderToString(
      <PersonnelToast
        notice={{ id: 1, title: 't', message: 'm', type: 'warning', action: 'report' }}
        onDismiss={() => {}}
        onOpenReport={() => {}}
      />,
    )

    expect(html).toContain('role="status"')
  })

  it('keeps the card operable by keyboard and labelled with the subject identity', () => {
    const html = renderToString(
      <PersonnelCard person={person} onView={() => {}} onEdit={() => {}} onDelete={() => {}} />,
    )

    expect(html).toContain('role="button"')
    expect(html).toContain('tabindex="0"')
    expect(html).toContain('aria-label="张三 (')
  })

  it('renders the modern SaaS data table with accessible columns and person identifiers', () => {
    const html = renderToString(
      <PersonnelTable
        items={[person]}
        selectedIds={new Set(['S-001'])}
        onToggleSelect={() => {}}
        onToggleSelectAll={() => {}}
        isAllSelected={true}
        onView={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
      />,
    )

    expect(html).toContain('<table')
    expect(html).toContain('#S-001')
    expect(html).toContain('张三')
    expect(html).toContain('安保部')
  })

  it('renders the floating batch action bar when selections exist', () => {
    const html = renderToString(
      <PersonnelBatchBar
        selectedCount={3}
        isAllSelected={false}
        onToggleSelectAll={() => {}}
        onClearSelection={() => {}}
        onBatchDelete={() => {}}
      />,
    )

    expect(html).toContain('role="toolbar"')
    expect(html).toContain('3')
  })

  it('renders the batch deletion confirmation modal with identity tags', () => {
    const html = renderToString(
      <BatchDeleteModal
        isOpen
        targets={[person]}
        isDeleting={false}
        onClose={() => {}}
        onConfirm={() => {}}
      />,
    )

    expect(html).toContain('role="alertdialog"')
    expect(html).toContain('张三')
    expect(html).toContain('#S-001')
  })
})

describe('Personnel bulk import modal', () => {
  it('accepts archives by naming convention and rejects other file types', () => {
    expect(isSupportedArchiveName('staff.zip')).toBe(true)
    expect(isSupportedArchiveName('STAFF.ZIP')).toBe(true)
    expect(isSupportedArchiveName('staff.tar.gz')).toBe(true)
    expect(isSupportedArchiveName('staff.tgz')).toBe(true)
    expect(isSupportedArchiveName('staff.tar')).toBe(true)
    expect(isSupportedArchiveName('staff.rar')).toBe(false)
    expect(isSupportedArchiveName('manifest.csv')).toBe(false)
  })

  it('treats only completed/failed/cancelled as terminal states', () => {
    expect(isTerminalImportStatus('idle')).toBe(false)
    expect(isTerminalImportStatus('running')).toBe(false)
    expect(isTerminalImportStatus('completed')).toBe(true)
    expect(isTerminalImportStatus('failed')).toBe(true)
    expect(isTerminalImportStatus('cancelled')).toBe(true)
  })

  it('guides the operator through both packaging styles before uploading', () => {
    const html = renderToString(
      <PersonnelImportModal
        isOpen
        progress={null}
        isStarting={false}
        isCancelling={false}
        maxArchiveMb={100}
        onClose={() => {}}
        onStart={() => {}}
        onCancelTask={() => {}}
      />,
    )

    expect(html).toContain('role="dialog"')
    expect(html).toContain('aria-modal="true"')
    expect(html).toContain('aria-labelledby=')
    expect(html).toContain('100')
    expect(html).toContain('manifest.csv')
    expect(html).not.toMatch(/(?:emerald|rose|amber)-\d+/)
  })

  it('reports running progress through an accessible progressbar with live counters', () => {
    const html = renderToString(
      <PersonnelImportModal
        isOpen
        initialMode="progress"
        progress={{
          taskId: 'task-1',
          status: 'running',
          total: 8,
          processed: 2,
          succeeded: 1,
          failed: 1,
          currentName: '张三',
          failures: [],
        }}
        isStarting={false}
        isCancelling={false}
        maxArchiveMb={100}
        onClose={() => {}}
        onStart={() => {}}
        onCancelTask={() => {}}
      />,
    )

    expect(html).toContain('role="progressbar"')
    expect(html).toContain('aria-valuenow="25"')
    expect(html).toContain('张三')
  })

  it('lists per-person failure reasons with their attribution tags in the report', () => {
    const html = renderToString(
      <PersonnelImportModal
        isOpen
        initialMode="report"
        progress={{
          taskId: 'task-2',
          status: 'completed',
          total: 3,
          processed: 3,
          succeeded: 2,
          failed: 1,
          failures: [
            {
              name: '李四',
              subjectId: 'EMP002',
              kind: 'clash',
              reason: '与底库人员「王五」样本高度相似',
              skippedPhotos: 2,
            },
          ],
        }}
        isStarting={false}
        isCancelling={false}
        maxArchiveMb={100}
        onClose={() => {}}
        onStart={() => {}}
        onCancelTask={() => {}}
      />,
    )

    expect(html).toContain('李四')
    expect(html).toContain('EMP002')
    expect(html).toContain('与底库人员「王五」样本高度相似')
    // 失败明细默认展开按钮可达，且复制行动项存在
    expect(html).toContain('aria-expanded')
  })

  it('surfaces a start failure inline rather than silently returning to upload', () => {
    const html = renderToString(
      <PersonnelImportModal
        isOpen
        progress={null}
        isStarting={false}
        isCancelling={false}
        error="当前有底库维护任务正在执行，请稍后再试"
        maxArchiveMb={100}
        onClose={() => {}}
        onStart={() => {}}
        onCancelTask={() => {}}
      />,
    )

    expect(html).toContain('role="alert"')
    expect(html).toContain('当前有底库维护任务正在执行')
  })

  it('surfaces an error when archive size exceeds the limit', () => {
    const html = renderToString(
      <PersonnelImportModal
        isOpen
        progress={null}
        isStarting={false}
        isCancelling={false}
        error="导入归档体积超出上限 (100 MB)，请拆分后分批导入"
        maxArchiveMb={100}
        onClose={() => {}}
        onStart={() => {}}
        onCancelTask={() => {}}
      />,
    )

    expect(html).toContain('role="alert"')
    expect(html).toContain('100 MB')
  })
})
