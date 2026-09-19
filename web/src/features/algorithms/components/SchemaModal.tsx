import React, { useMemo, useState } from 'react'
import { Code2, Copy, FileJson, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { useDismissStack } from '@/hooks/use-dismiss-stack'
import { copyToClipboard } from '@/lib/utils'
import type { AlgorithmItem } from '@/types'
import { activeVersionItem } from '../algoFilters'
import { Overlay } from './Overlay'

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
 * 参数较多的算法（十几个可调项）在平板上翻表很难定位，因此提供属性过滤；
 * 过滤只影响表格视图，原始 JSON 始终可整体复制，避免出现「复制的是过滤后子集」的误解。
 */
export function SchemaModal({ isOpen, algorithm, onClose }: SchemaModalProps): React.ReactElement {
  const { t } = useTranslation('algo')
  const [showRaw, setShowRaw] = useState(false)
  const [copied, setCopied] = useState(false)
  const [propertyQuery, setPropertyQuery] = useState('')

  useDismissStack(isOpen, onClose)

  const activeVersion = algorithm ? activeVersionItem(algorithm) : undefined

  const schemaObj = useMemo(
    () => (activeVersion?.configSchema as Record<string, unknown>) ?? {},
    [activeVersion],
  )

  const properties: SchemaProperty[] = useMemo(() => {
    const propertiesObj = (schemaObj.properties as Record<string, Record<string, unknown>>) ?? {}
    return Object.entries(propertiesObj).map(([key, value]) => ({
      key,
      type: String(value.type || 'unknown'),
      defaultVal: value.default !== undefined ? JSON.stringify(value.default) : '-',
      description: String(value.description || '-'),
    }))
  }, [schemaObj])

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
    <Overlay
      isOpen={isOpen && Boolean(algorithm)}
      onClose={onClose}
      ariaLabel={`${t('schema.title')} - ${algorithm?.name ?? ''}`}
      variant="modal"
      panelClassName="max-w-2xl"
    >
      <div className="flex items-center justify-between border-b border-[var(--border)] pb-4">
        <div className="flex min-w-0 items-center gap-2.5">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-[var(--accent-soft)] text-[var(--accent)]">
            <Code2 className="h-5 w-5" />
          </div>
          <div className="min-w-0">
            <h3 className="truncate text-sm font-bold text-[var(--text-primary)]">
              {t('schema.title')} · {algorithm?.name ?? ''}
            </h3>
            <p className="text-[11px] text-[var(--text-muted)]">{t('schema.subtitle')}</p>
          </div>
        </div>
        <button
          type="button"
          data-autofocus
          onClick={onClose}
          aria-label={t('actions.close')}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-[var(--text-muted)] transition-colors hover:bg-[var(--accent-soft)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
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
            <FileJson className="h-3.5 w-3.5" />
            <span>{t('schema.rawJson')}</span>
          </button>
        </div>

        <div className="flex items-center gap-2">
          {!showRaw && properties.length > PROPERTY_SEARCH_THRESHOLD && (
            <div className="group/search relative">
              <Search className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-[var(--text-muted)] group-focus-within/search:text-[var(--accent)]" />
              <input
                type="text"
                value={propertyQuery}
                onChange={(event) => setPropertyQuery(event.target.value)}
                placeholder={t('schema.filterPlaceholder')}
                aria-label={t('schema.filterPlaceholder')}
                className="w-44 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] py-1.5 pr-8 pl-8 text-xs text-[var(--text-primary)] transition-colors hover:border-[var(--border-strong)] focus:border-[var(--accent)] focus:outline-hidden"
              />
              {propertyQuery && (
                <button
                  type="button"
                  onClick={() => setPropertyQuery('')}
                  aria-label={t('filter.clearSearch')}
                  className="absolute top-1/2 right-1 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded-md text-[var(--text-muted)] hover:text-[var(--text-primary)] focus-visible:ring-2 focus-visible:ring-[var(--ring)] focus-visible:outline-hidden"
                >
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          )}

          <button
            type="button"
            onClick={handleCopy}
            className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs font-medium text-[var(--accent)] transition-colors hover:bg-[var(--accent-soft)]"
          >
            <Copy className="h-3.5 w-3.5" />
            <span>{copied ? t('schema.copied') : t('schema.copyJson')}</span>
          </button>
        </div>
      </div>

      <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
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

      <div className="mt-6 flex justify-end border-t border-[var(--border)] pt-4">
        <button
          type="button"
          onClick={onClose}
          className="rounded-xl border border-[var(--border)] px-4 py-2 text-xs font-medium text-[var(--text-secondary)] transition-colors hover:bg-[var(--bg-secondary)] hover:text-[var(--text-primary)]"
        >
          {t('actions.close')}
        </button>
      </div>
    </Overlay>
  )
}
