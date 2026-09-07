import type {
  AlgoManifest,
  DetectionLineDirection,
  DetectionPoint,
  DetectionRule,
  DetectionRuleRole,
} from '@/types'

export type ToolMode = 'select' | 'rect' | 'polygon' | 'roi' | 'line' | 'mask' | 'precrop'

export interface ExtendedRule extends DetectionRule {
  id: string
  name: string
  visible: boolean
  boundAlgo?: string
  targetClasses?: string[]
  color?: string
}

export interface RuleColorTheme {
  stroke: string
  selectedStroke: string
  fill: string
  handleBg: string
  handleRing: string
  badgeText: string
  badgeBg: string
}

export const DEFAULT_ALGO_PACKAGES: AlgoManifest[] = [
  {
    algorithmId: 'general_detection',
    name: '通用人体与车辆检测器',
    version: '1.0.0',
    author: 'Heimdall AI Team',
    description: '高能效实时多类目标检测，适配 ANE/NPU 零拷贝管线',
    category: 'detection',
    supportedPlatforms: ['macos-arm64', 'linux-rknn'],
    classes: ['person', 'car', 'bicycle', 'motorcycle', 'bus', 'truck'],
  },
]

export const DEFAULT_SANDBOX_STEPS: string[] = [
  '1. 路径防穿透与目录结构检查',
  '2. SHA256 完整性与安全指纹校验',
  '3. 解析 Manifest 与平台拓扑匹配',
  '4. Config Schema 参数格式校验',
  '5. 派生隔离子进程与超时守护',
  '6. 算法库 C ABI 导出符号核对',
  '7. 真实前向推理自测与内存复核',
]

export const CLASS_NAME_MAP: Record<string, { zh: string; en: string }> = {
  person: { zh: '行人', en: 'Person' },
  car: { zh: '机动车', en: 'Car' },
  bicycle: { zh: '单车/非机动车', en: 'Bicycle' },
  motorcycle: { zh: '摩托车/电动车', en: 'Motorcycle' },
  bus: { zh: '客车/大巴', en: 'Bus' },
  truck: { zh: '卡车/货车', en: 'Truck' },
  dog: { zh: '犬只', en: 'Dog' },
  cat: { zh: '猫', en: 'Cat' },
  helmet: { zh: '安全帽', en: 'Helmet' },
  vest: { zh: '反光背心', en: 'Reflective Vest' },
  fire: { zh: '火焰', en: 'Fire' },
  smoke: { zh: '烟雾', en: 'Smoke' },
  face: { zh: '人脸', en: 'Face' },
  license_plate: { zh: '车牌', en: 'License Plate' },
}

export function getLocalizedClassName(cls: string, lang: string): string {
  const meta = CLASS_NAME_MAP[cls.toLowerCase()]
  if (!meta) return cls
  return lang.startsWith('zh') ? `${meta.zh} (${cls})` : meta.en
}

export function isPointInPolygon(pt: DetectionPoint, polygon: DetectionPoint[]): boolean {
  if (polygon.length < 3) return false
  let inside = false
  for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
    const xi = polygon[i].x
    const yi = polygon[i].y
    const xj = polygon[j].x
    const yj = polygon[j].y
    const intersect = yi > pt.y !== yj > pt.y && pt.x < ((xj - xi) * (pt.y - yi)) / (yj - yi) + xi
    if (intersect) inside = !inside
  }
  return inside
}

export function isPointNearLine(
  pt: DetectionPoint,
  p1: DetectionPoint,
  p2: DetectionPoint,
  threshold = 0.03,
): boolean {
  const l2 = (p2.x - p1.x) ** 2 + (p2.y - p1.y) ** 2
  if (l2 === 0) return Math.hypot(pt.x - p1.x, pt.y - p1.y) < threshold
  let t = ((pt.x - p1.x) * (p2.x - p1.x) + (pt.y - p1.y) * (p2.y - p1.y)) / l2
  t = Math.max(0, Math.min(1, t))
  const projX = p1.x + t * (p2.x - p1.x)
  const projY = p1.y + t * (p2.y - p1.y)
  return Math.hypot(pt.x - projX, pt.y - projY) < threshold
}

// 预设高对比度工程美学调色板
export const ROI_COLOR_PALETTES: RuleColorTheme[] = [
  {
    stroke: '#00d5e8',
    selectedStroke: '#00f2fe',
    fill: 'rgba(0, 213, 232, 0.16)',
    handleBg: 'bg-cyan-400 border-black/80',
    handleRing: 'ring-cyan-400/50',
    badgeText: 'text-cyan-400',
    badgeBg: 'bg-cyan-500/10',
  },
  {
    stroke: '#f59e0b',
    selectedStroke: '#fbbf24',
    fill: 'rgba(245, 158, 11, 0.16)',
    handleBg: 'bg-amber-400 border-black/80',
    handleRing: 'ring-amber-400/50',
    badgeText: 'text-amber-400',
    badgeBg: 'bg-amber-500/10',
  },
  {
    stroke: '#a855f7',
    selectedStroke: '#c084fc',
    fill: 'rgba(168, 85, 247, 0.16)',
    handleBg: 'bg-purple-400 border-black/80',
    handleRing: 'ring-purple-400/50',
    badgeText: 'text-purple-400',
    badgeBg: 'bg-purple-500/10',
  },
  {
    stroke: '#f43f5e',
    selectedStroke: '#fb7185',
    fill: 'rgba(244, 63, 94, 0.16)',
    handleBg: 'bg-rose-400 border-black/80',
    handleRing: 'ring-rose-400/50',
    badgeText: 'text-rose-400',
    badgeBg: 'bg-rose-500/10',
  },
  {
    stroke: '#6366f1',
    selectedStroke: '#818cf8',
    fill: 'rgba(99, 102, 241, 0.16)',
    handleBg: 'bg-indigo-400 border-black/80',
    handleRing: 'ring-indigo-400/50',
    badgeText: 'text-indigo-400',
    badgeBg: 'bg-indigo-500/10',
  },
]

export const LINE_COLOR_THEME: RuleColorTheme = {
  stroke: '#10b981',
  selectedStroke: '#34d399',
  fill: 'rgba(16, 185, 129, 0.16)',
  handleBg: 'bg-emerald-400 border-black/80',
  handleRing: 'ring-emerald-400/50',
  badgeText: 'text-emerald-400',
  badgeBg: 'bg-emerald-500/10',
}

export const MASK_COLOR_THEME: RuleColorTheme = {
  stroke: '#64748b',
  selectedStroke: '#94a3b8',
  fill: 'rgba(15, 23, 42, 0.65)',
  handleBg: 'bg-slate-400 border-black/80',
  handleRing: 'ring-slate-400/50',
  badgeText: 'text-slate-400',
  badgeBg: 'bg-slate-500/10',
}

export function getRuleTheme(rule: ExtendedRule, index = 0): RuleColorTheme {
  if (rule.role === 'line') return LINE_COLOR_THEME
  if (rule.role === 'mask') return MASK_COLOR_THEME
  if (rule.color) {
    const found = ROI_COLOR_PALETTES.find((p) => p.stroke === rule.color)
    if (found) return found
  }
  return ROI_COLOR_PALETTES[index % ROI_COLOR_PALETTES.length]
}

export function getToolTheme(tool: ToolMode, currentRoiCount = 0): RuleColorTheme {
  if (tool === 'line') return LINE_COLOR_THEME
  if (tool === 'mask') return MASK_COLOR_THEME
  if (tool === 'precrop') return ROI_COLOR_PALETTES[1] // amber
  return ROI_COLOR_PALETTES[currentRoiCount % ROI_COLOR_PALETTES.length]
}

export function getInitialRuleColor(role: DetectionRuleRole, roiIndex: number): string {
  switch (role) {
    case 'roi':
      return ROI_COLOR_PALETTES[roiIndex % ROI_COLOR_PALETTES.length].stroke
    case 'line':
      return LINE_COLOR_THEME.stroke
    case 'mask':
      return MASK_COLOR_THEME.stroke
  }
}

export function getDefaultRuleName(role: DetectionRuleRole, index: number): string {
  switch (role) {
    case 'roi':
      return `入侵防区 ${index}`
    case 'line':
      return `越界绊线 ${index}`
    case 'mask':
      return `屏蔽遮罩 ${index}`
    default:
      return `规则 ${index}`
  }
}

export function getLineMarkerEnd(direction?: DetectionLineDirection): string | undefined {
  if (direction === 'a_to_b') {
    return 'url(#line-arrow-a-to-b)'
  }
  if (direction === 'b_to_a') {
    return 'url(#line-arrow-b-to-a)'
  }
  return undefined
}

export function getDirectionLabel(dir: string, t: (key: string) => string): string {
  if (dir === 'both') return t('inspector.dirBoth')
  if (dir === 'a_to_b') return t('inspector.dirAtoB')
  return t('inspector.dirBtoA')
}
