import { renderToString } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { PersonnelItem } from '../../types'
import { BatchDeleteModal } from './components/BatchDeleteModal'
import { DeleteConfirmModal } from './components/DeleteConfirmModal'
import { PersonnelBatchBar } from './components/PersonnelBatchBar'
import { PersonnelCard } from './components/PersonnelCard'
import { PersonnelModal } from './components/PersonnelModal'
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
