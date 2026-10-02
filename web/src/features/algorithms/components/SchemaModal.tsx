import React, { useId, useMemo, useState } from 'react'
import { Code2, Copy, FileJson } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { ModalFormHeader } from '@/components/ui/ModalFormHeader'
import { ModalOverlay } from '@/components/ui/ModalOverlay'
import { SearchInput } from '@/components/ui/SearchInput'
import { extractConfigProperties } from '@/lib/algoConfigSchema'
import { copyToClipboard } from '@/lib/utils'
import type { AlgorithmItem } from '@/types'
import { activeVersionItem } from '../algoFilters'

export interface SchemaModalProps {
  isOpen: boolean
  algorithm: AlgorithmItem | null
  onClose: () => void
}

interface SchemaProperty {
  key: string
  type: string
  defaultVal: string
  description: string
}

/** 属性数量超过该阈值时才出现过滤框，避免少数几项时还多一层交互 */
const PROPERTY_SEARCH_THRESHOLD = 6

/**
 * 参数配置规范查看器。
 *
 * 外壳、遮罩、焦点陷阱与 ESC 由 [ModalOverlay](../../../components/ui/ModalOverlay.tsx)
 * 承担；标题、唯一滚动区与页脚分别复用 `ModalFormHeader` / `.modal-form-content` /
 * `.modal-form-footer`，与同域的上传弹窗保持同一套结构，不另起视觉层。
 *
 * 因为存在可见标题节点，无障碍名称走 `aria-labelledby` / `aria-describedby`
 * （`ariaLabel` 在有 `ariaLabelledBy` 时不会渲染，重复传属死参数）。
 *
 * 参数较多的算法（十几个可调项）在平板上翻表很难定位，因此提供属性过滤；
 * 过滤只影响表格视图，原始 JSON 始终可整体复制，避免出现「复制的是过滤后子集」的误解。
 */
export function SchemaModal({ isOpen, algorithm, onClose }: SchemaModalProps): React.ReactElement {
  const { t } = useTranslation('algo')
  const titleId = useId()
  const descriptionId = useId()
  const [showRaw, setShowRaw] = useState(false)
  const [copied, setCopied] = useState(false)
  const [propertyQuery, setPropertyQuery] = useState('')

  const activeVersion = algorithm ? activeVersionItem(algorithm) : undefined

  const schemaObj = useMemo(
    () => (activeVersion?.configSchema as Record<string, unknown>) ?? {},
    [activeVersion],
  )

  // 复用共享解析：与 AlgoParamDrawer 的参数一览同源，避免两处结构校验各自漂移
  const properties: SchemaProperty[] = useMemo(() => {
    const propertiesObj = extractConfigProperties(activeVersion)
    return Object.entries(propertiesObj).map(([key, value]) => ({
      key,
      type: String(value.type || 'unknown'),
      defaultVal: value.default !== undefined ? JSON.stringify(value.default) : '-',
      description: String(value.description || '-'),
    }))
  }, [activeVersion])

  const visibleProperties = useMemo(() => {
    const query = propertyQuery.trim().toLowerCase()
    if (!query) return properties
    return properties.filter(
      (property) =>
        property.key.toLowerCase().includes(query) ||
        property.description.toLowerCase().includes(query),
    )
  }, [properties, propertyQuery])

  const rawJsonString = useMemo(() => JSON.stringify(schemaObj, null, 2), [schemaObj])

  const handleCopy = async () => {
    const success = await copyToClipboard(rawJsonString)
    if (success) {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    }
  }

  return (
    <ModalOverlay
      isOpen={isOpen && Boolean(algorithm)}
      onClose={onClose}
      ariaLabelledBy={titleId}
      ariaDescribedBy={descriptionId}
      surface="solid"
      panelClassName="modal-surface--form modal-surface--medium p-0"
    >
      <ModalFormHeader
        icon={Code2}
        title={`${t('schema.title')} · ${algorithm?.name ?? ''}`}
        titleId={titleId}
        description={t('schema.subtitle')}
        descriptionId={descriptionId}
        closeLabel={t('actions.close')}
        onClose={onClose}
      />

      {/* 工具栏：视图切换 / 属性过滤 / 复制。过滤只作用于表格视图。 */}
      <div className="modal-form-toolbar">
        <div
          role="group"
          aria-label={t('schema.title')}
          className="flex rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-0.5 text-xs"
        >
          <button
            type="button"
            aria-pressed={!showRaw}
            onClick={() => setShowRaw(false)}
            className={`rounded-lg px-3 py-1.5 font-medium transition-all ${
              !showRaw
                ? 'bg-[var(--accent)] text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
          >
            {t('schema.tableView')}
          </button>
          <button
            type="button"
            aria-pressed={showRaw}
            onClick={() => setShowRaw(true)}
            className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 font-medium transition-all ${
              showRaw
                ? 'bg-[var(--accent)] text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
          >
            <FileJson className="h-3.5 w-3.5" aria-hidden="true" />
            <span>{t('schema.rawJson')}</span>
          </button>
        </div>

        <div className="ms-auto flex items-center gap-2">
          {!showRaw && properties.length > PROPERTY_SEARCH_THRESHOLD && (
            <SearchInput
              sizeVariant="compact"
              value={propertyQuery}
              onChange={setPropertyQuery}
              onClear={() => setPropertyQuery('')}
              placeholder={t('schema.filterPlaceholder')}
              aria-label={t('schema.filterPlaceholder')}
              clearAriaLabel={t('filter.clearSearch')}
              containerClassName="w-44"
            />
          )}

          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs font-medium text-[var(--accent)] transition-colors hover:bg-[var(--accent-soft)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
          >
            <Copy className="h-3.5 w-3.5" aria-hidden="true" />
            <span>{copied ? t('schema.copied') : t('schema.copyJson')}</span>
          </button>
        </div>
      </div>

      {/* 唯一滚动区：头/工具栏/页脚固定在滚动区外 */}
      <div className="modal-form-content">
        {showRaw ? (
          <pre className="font-data rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-4 text-xs leading-relaxed text-[var(--text-primary)]">
            {rawJsonString}
          </pre>
        ) : properties.length === 0 ? (
          <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-8 text-center text-xs text-[var(--text-muted)]">
            {t('schema.noProperties')}
          </div>
        ) : visibleProperties.length === 0 ? (
          <div className="rounded-2xl border border-[var(--border)] bg-[var(--bg-secondary)] p-8 text-center text-xs text-[var(--text-muted)]">
            {t('schema.noMatchedProperties')}
          </div>
        ) : (
          <div className="overflow-hidden rounded-2xl border border-[var(--border)]">
            <table className="w-full text-left text-xs">
              <caption className="sr-only">
                {t('schema.title')} · {algorithm?.name ?? ''}
              </caption>
              <thead className="border-b border-[var(--border)] bg-[var(--bg-secondary)] text-[11px] text-[var(--text-muted)]">
                <tr>
                  <th scope="col" className="px-4 py-2.5 font-semibold">
                    {t('schema.propertyName')}
                  </th>
                  <th scope="col" className="px-4 py-2.5 font-semibold">
                    {t('schema.type')}
                  </th>
                  <th scope="col" className="px-4 py-2.5 font-semibold">
                    {t('schema.default')}
                  </th>
                  <th scope="col" className="px-4 py-2.5 font-semibold">
                    {t('schema.description')}
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-[var(--border)]">
                {visibleProperties.map((property) => (
                  <tr key={property.key} className="hover:bg-[var(--bg-secondary)]/50">
                    <td className="font-data px-4 py-2.5 font-bold text-[var(--accent)]">
                      {property.key}
                    </td>
                    <td className="font-data px-4 py-2.5 text-[var(--text-secondary)]">
                      {property.type}
                    </td>
                    <td className="font-data px-4 py-2.5 text-[var(--text-muted)]">
                      {property.defaultVal}
                    </td>
                    <td className="px-4 py-2.5 text-[var(--text-secondary)]">
                      {property.description}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <div className="modal-form-footer">
        <button
          type="button"
          onClick={onClose}
          className="modal-form-button modal-form-button--secondary"
        >
          {t('actions.close')}
        </button>
      </div>
    </ModalOverlay>
  )
}
