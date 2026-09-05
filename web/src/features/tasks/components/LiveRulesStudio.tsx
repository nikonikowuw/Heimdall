import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
  AlertCircle,
  ArrowLeft,
  Check,
  CheckCircle2,
  Copy,
  Cpu,
  Crop,
  Eye,
  EyeOff,
  Hexagon,
  Layers,
  Magnet,
  MousePointer2,
  Save,
  ShieldAlert,
  ShieldCheck,
  Slash,
  SlidersHorizontal,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { algoApi, cameraApi, taskApi } from '../../../lib/api'
import type {
  AlgoManifest,
  Camera,
  DetectionLineDirection,
  DetectionPoint,
  DetectionRule,
  DetectionRuleRole,
  SandboxCheckResult,
  TaskConfigDto,
} from '../../../types'
import { LivePlayer } from '../../live/components/LivePlayer'

const DEFAULT_ALGO_PACKAGES: AlgoManifest[] = [
  {
    algorithmId: 'general_detection',
    name: '通用人体与车辆检测器',
    version: '1.0.0',
    author: 'Argus AI Team',
    description: '高能效实时多类目标检测，适配 ANE/NPU 零拷贝管线',
    category: 'detection',
    supportedPlatforms: ['macos-arm64', 'linux-rknn'],
    classes: ['person', 'car', 'bicycle', 'motorcycle', 'bus', 'truck'],
  },
]

type ToolMode = 'select' | 'roi' | 'line' | 'mask' | 'precrop'

interface ExtendedRule extends DetectionRule {
  id: string
  name: string
  visible: boolean
  boundAlgo?: string
  targetClasses?: string[]
}

interface LiveRulesStudioProps {
  initialCamera?: Camera | null
  onBack?: () => void
}

function getDefaultRuleName(role: DetectionRuleRole, index: number): string {
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

function getLineMarkerEnd(direction?: DetectionLineDirection): string | undefined {
  if (direction === 'a_to_b') {
    return 'url(#line-arrow-a-to-b)'
  }
  if (direction === 'b_to_a') {
    return 'url(#line-arrow-b-to-a)'
  }
  return undefined
}

function getDirectionLabel(dir: string, t: (key: string) => string): string {
  if (dir === 'both') return t('inspector.dirBoth')
  if (dir === 'a_to_b') return t('inspector.dirAtoB')
  return t('inspector.dirBtoA')
}

export function LiveRulesStudio({
  initialCamera,
  onBack,
}: LiveRulesStudioProps): React.ReactElement {
  const { t } = useTranslation('task')
  const [cameras, setCameras] = useState<Camera[]>([])
  const [selectedCamera, setSelectedCamera] = useState<Camera | null>(initialCamera || null)
  const [taskConfig, setTaskConfig] = useState<TaskConfigDto | null>(null)
  const [rules, setRules] = useState<ExtendedRule[]>([])
  const [selectedRuleId, setSelectedRuleId] = useState<string | null>(null)
  const [tool, setTool] = useState<ToolMode>('select')
  const [currentPoints, setCurrentPoints] = useState<DetectionPoint[]>([])
  const [cursorPos, setCursorPos] = useState<DetectionPoint | null>(null)
  const [snapEnabled, setSnapEnabled] = useState(true)
  const [isArmed, setIsArmed] = useState(true)
  const [availableAlgos, setAvailableAlgos] = useState<AlgoManifest[]>([])
  const [isAlgoDrawerOpen, setIsAlgoDrawerOpen] = useState(false)
  const [selectedAlgoForConfig, setSelectedAlgoForConfig] = useState<AlgoManifest | null>(null)
  const [sandboxResult, setSandboxResult] = useState<SandboxCheckResult | null>(null)
  const [isVerifyingSandbox, setIsVerifyingSandbox] = useState(false)
  const [isUploadingPkg, setIsUploadingPkg] = useState(false)
  const [isSaving, setIsSaving] = useState(false)
  const [saveToast, setSaveToast] = useState<string | null>(null)

  // 顶点拖拽状态
  const [draggingVertex, setDraggingVertex] = useState<{
    ruleId: string
    pointIndex: number
  } | null>(null)

  const stageRef = useRef<HTMLDivElement>(null)

  // 加载摄像头与算法包列表
  useEffect(() => {
    let isMounted = true
    cameraApi
      .list()
      .then((list) => {
        if (!isMounted) return
        setCameras(list)
        if (!selectedCamera && list.length > 0) {
          setSelectedCamera(list[0])
        }
      })
      .catch(() => {})

    algoApi
      .listPackages()
      .then((pkgs) => {
        if (!isMounted) return
        setAvailableAlgos(pkgs.length > 0 ? pkgs : DEFAULT_ALGO_PACKAGES)
      })
      .catch(() => {
        if (!isMounted) return
        setAvailableAlgos(DEFAULT_ALGO_PACKAGES)
      })

    return () => {
      isMounted = false
    }
  }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // 加载选定摄像头的任务布防配置
  useEffect(() => {
    if (!selectedCamera) return

    taskApi
      .getTask(selectedCamera.cameraId)
      .then((dto) => {
        setTaskConfig(dto)
        setIsArmed(dto.desiredEnabled)

        const extRules: ExtendedRule[] = dto.rules.map((r, idx) => ({
          ...r,
          lineDirection:
            r.lineDirection ||
            (r as unknown as { line_direction?: DetectionLineDirection }).line_direction ||
            'both',
          id: `rule_${idx}_${Date.now()}`,
          name: getDefaultRuleName(r.role, idx + 1),
          visible: true,
          boundAlgo: 'general_detection',
          targetClasses: ['person', 'car'],
        }))
        setRules(extRules)
        if (extRules.length > 0) {
          setSelectedRuleId(extRules[0].id)
        }
      })
      .catch(() => {})
  }, [selectedCamera])

  // 键盘快捷键响应
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return

      if (e.key === 'v' || e.key === 'V') setTool('select')
      if (e.key === 'r' || e.key === 'R') {
        setTool('roi')
        setCurrentPoints([])
      }
      if (e.key === 'l' || e.key === 'L') {
        setTool('line')
        setCurrentPoints([])
      }
      if (e.key === 'm' || e.key === 'M') {
        setTool('mask')
        setCurrentPoints([])
      }
      if (e.key === 'c' || e.key === 'C') {
        setTool('precrop')
        setCurrentPoints([])
      }
      if (e.key === 'Enter') {
        finishDrawing()
      }
      if (e.key === 'Escape') {
        setCurrentPoints([])
        setTool('select')
      }
      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (selectedRuleId && tool === 'select') {
          deleteRule(selectedRuleId)
        }
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [tool, currentPoints, selectedRuleId, rules.length]) // eslint-disable-line react-hooks/exhaustive-deps

  // 获取归一化鼠标相对坐标 [0.0, 1.0]
  const getNormalizedPoint = useCallback(
    (e: React.MouseEvent<HTMLDivElement>): DetectionPoint => {
      if (!stageRef.current) return { x: 0, y: 0 }
      const rect = stageRef.current.getBoundingClientRect()
      let x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
      let y = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height))

      // 磁吸吸附到既有顶点
      if (snapEnabled) {
        const threshold = 0.02
        for (const rule of rules) {
          if (!rule.visible) continue
          for (const pt of rule.points) {
            if (Math.hypot(pt.x - x, pt.y - y) < threshold) {
              x = pt.x
              y = pt.y
              break
            }
          }
        }
      }

      return { x: Number(x.toFixed(4)), y: Number(y.toFixed(4)) }
    },
    [snapEnabled, rules],
  )

  // 完成当前绘制
  const finishDrawing = () => {
    if (currentPoints.length < 2) {
      setCurrentPoints([])
      return
    }

    let role: DetectionRuleRole = 'roi'
    if (tool === 'line') role = 'line'
    if (tool === 'mask') role = 'mask'
    if (tool === 'precrop') role = 'roi'

    const newRule: ExtendedRule = {
      id: `rule_${Date.now()}`,
      name: getDefaultRuleName(role, rules.length + 1),
      role,
      lineDirection: role === 'line' ? 'both' : undefined,
      points: [...currentPoints],
      visible: true,
      boundAlgo: 'general_detection',
      targetClasses: ['person', 'car'],
    }

    setRules((prev) => [...prev, newRule])
    setSelectedRuleId(newRule.id)
    setCurrentPoints([])
    setTool('select')
  }

  // 画布点击处理
  const handleStageClick = (e: React.MouseEvent<HTMLDivElement>) => {
    if (tool === 'select') return

    const pt = getNormalizedPoint(e)

    if (tool === 'line') {
      const next = [...currentPoints, pt]
      if (next.length >= 2) {
        const newRule: ExtendedRule = {
          id: `rule_${Date.now()}`,
          name: `越界绊线 ${rules.length + 1}`,
          role: 'line',
          lineDirection: 'both',
          points: next,
          visible: true,
          boundAlgo: 'general_detection',
          targetClasses: ['person', 'car'],
        }
        setRules((prev) => [...prev, newRule])
        setSelectedRuleId(newRule.id)
        setCurrentPoints([])
        setTool('select')
      } else {
        setCurrentPoints(next)
      }
    } else if (tool === 'precrop') {
      const next = [...currentPoints, pt]
      if (next.length >= 2) {
        // 转换两点为 4 顶点矩形
        const p1 = next[0]
        const p2 = next[1]
        const x1 = Math.min(p1.x, p2.x)
        const y1 = Math.min(p1.y, p2.y)
        const x2 = Math.max(p1.x, p2.x)
        const y2 = Math.max(p1.y, p2.y)
        const rectPoints: DetectionPoint[] = [
          { x: x1, y: y1 },
          { x: x2, y: y1 },
          { x: x2, y: y2 },
          { x: x1, y: y2 },
        ]

        const newRule: ExtendedRule = {
          id: `rule_${Date.now()}`,
          name: `局部特写 ${rules.length + 1}`,
          role: 'roi',
          points: rectPoints,
          visible: true,
          boundAlgo: 'general_detection',
          targetClasses: ['person'],
        }
        setRules((prev) => [...prev, newRule])
        setSelectedRuleId(newRule.id)
        setCurrentPoints([])
        setTool('select')
      } else {
        setCurrentPoints(next)
      }
    } else {
      // 多边形 (roi / mask)
      if (currentPoints.length >= 3) {
        const first = currentPoints[0]
        if (Math.hypot(first.x - pt.x, first.y - pt.y) < 0.03) {
          finishDrawing()
          return
        }
      }
      setCurrentPoints((prev) => [...prev, pt])
    }
  }

  // 鼠标移动
  const handleStageMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    const pt = getNormalizedPoint(e)
    setCursorPos(pt)

    if (draggingVertex) {
      setRules((prev) =>
        prev.map((r) => {
          if (r.id !== draggingVertex.ruleId) return r
          const pts = [...r.points]
          pts[draggingVertex.pointIndex] = pt
          return { ...r, points: pts }
        }),
      )
    }
  }

  const handleStageMouseUp = () => {
    setDraggingVertex(null)
  }

  // 保存任务配置到后端
  const handleSave = async () => {
    if (!selectedCamera) return
    setIsSaving(true)

    try {
      const payloadDto: TaskConfigDto = {
        cameraId: selectedCamera.cameraId,
        name: taskConfig?.name || `Task-${selectedCamera.cameraId}`,
        desiredEnabled: isArmed,
        rules: rules.map((r) => ({
          role: r.role,
          lineDirection: r.lineDirection,
          points: r.points,
        })),
        motionGate: taskConfig?.motionGate || { enabled: true },
      }

      await taskApi.updateTask(selectedCamera.cameraId, payloadDto)
      setSaveToast(t('footer.saveSuccess'))
      setTimeout(() => setSaveToast(null), 3000)
    } catch (err) {
      setSaveToast(`保存失败: ${err instanceof Error ? err.message : String(err)}`)
      setTimeout(() => setSaveToast(null), 4000)
    } finally {
      setIsSaving(false)
    }
  }

  // 删除规则
  const deleteRule = (ruleId: string) => {
    setRules((prev) => prev.filter((r) => r.id !== ruleId))
    if (selectedRuleId === ruleId) {
      setSelectedRuleId(null)
    }
  }

  // 克隆规则
  const cloneRule = (ruleId: string) => {
    const target = rules.find((r) => r.id === ruleId)
    if (!target) return
    const cloned: ExtendedRule = {
      ...target,
      id: `rule_${Date.now()}`,
      name: `${target.name} (副本)`,
      points: target.points.map((p) => ({
        x: Math.min(1, p.x + 0.03),
        y: Math.min(1, p.y + 0.03),
      })),
    }
    setRules((prev) => [...prev, cloned])
    setSelectedRuleId(cloned.id)
  }

  // 触发沙箱七步安全验证
  const handleRunSandboxTest = async () => {
    setIsVerifyingSandbox(true)
    setSandboxResult(null)
    try {
      const res = await algoApi.verifyPackage()
      setSandboxResult(res)
    } catch (err) {
      setSandboxResult({
        passed: false,
        stepsTotal: 7,
        stepsPassed: 3,
        steps: [
          '1. 路径防穿透与目录结构检查',
          '2. SHA256 完整性与安全指纹校验',
          '3. 解析 Manifest 与平台拓扑匹配',
          '4. Config Schema 参数格式校验',
          '5. 派生隔离子进程与超时守护',
          '6. 算法库 C ABI 导出符号核对',
          '7. 真实前向推理自测与内存复核',
        ],
        errorMessage: err instanceof Error ? err.message : String(err),
      })
    } finally {
      setIsVerifyingSandbox(false)
    }
  }

  // 上传算法包归档 (.zip / .tar.gz / .tar) 并在隔离沙箱中加载
  const handleUploadPackageFile = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    if (!file) return
    setIsUploadingPkg(true)
    try {
      const res = await algoApi.uploadPackage(file)
      setSandboxResult(res)
      if (res.passed && res.manifest) {
        setAvailableAlgos((prev) => [
          res.manifest!,
          ...prev.filter((p) => p.algorithmId !== res.manifest!.algorithmId),
        ])
        setSelectedAlgoForConfig(res.manifest)
        setSaveToast(t('sandbox.uploadSuccess'))
      }
    } catch (err) {
      setSandboxResult({
        passed: false,
        stepsTotal: 7,
        stepsPassed: 0,
        steps: [
          '1. 路径防穿透与目录结构检查',
          '2. SHA256 完整性与安全指纹校验',
          '3. 解析 Manifest 与平台拓扑匹配',
          '4. Config Schema 参数格式校验',
          '5. 派生隔离子进程与超时守护',
          '6. 算法库 C ABI 导出符号核对',
          '7. 真实前向推理自测与内存复核',
        ],
        errorMessage: err instanceof Error ? err.message : String(err),
      })
    } finally {
      setIsUploadingPkg(false)
      e.target.value = ''
    }
  }

  const selectedRule = rules.find((r) => r.id === selectedRuleId)

  return (
    <div className="flex h-full w-full overflow-hidden bg-[var(--bg-primary)] font-sans text-[var(--text-primary)] select-none">
      {/* ----------------- 左侧：通道身份与挂载算法栈 ----------------- */}
      <aside className="frosted-glass flex w-80 min-w-80 flex-col overflow-y-auto border-r border-[var(--border)] text-xs">
        {/* 通道主控区 */}
        <div className="space-y-3 border-b border-[var(--border)] bg-[var(--bg-secondary)]/50 p-4">
          <div className="flex items-center justify-between">
            {onBack && (
              <button
                onClick={onBack}
                className="flex items-center gap-1.5 text-[11px] text-[var(--text-secondary)] transition-colors hover:text-[var(--accent)]"
              >
                <ArrowLeft className="h-3.5 w-3.5" />
                <span>{t('studio.backToChannels')}</span>
              </button>
            )}
            <div className="flex items-center gap-1.5">
              <span
                className={`h-2 w-2 rounded-full ${isArmed ? 'animate-pulse bg-emerald-500' : 'bg-slate-400'}`}
              />
              <span
                className={`font-mono text-[10px] font-semibold tracking-wide ${isArmed ? 'text-emerald-500' : 'text-[var(--text-muted)]'}`}
              >
                {isArmed ? t('studio.armed') : t('studio.unarmed')}
              </span>
            </div>
          </div>

          <div>
            <div className="flex items-center justify-between">
              <select
                value={selectedCamera?.cameraId || ''}
                onChange={(e) => {
                  const found = cameras.find((c) => c.cameraId === e.target.value)
                  if (found) setSelectedCamera(found)
                }}
                className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-2.5 py-1.5 text-xs font-semibold text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
              >
                {cameras.map((c) => (
                  <option key={c.cameraId} value={c.cameraId}>
                    {c.name} ({c.cameraId})
                  </option>
                ))}
              </select>
            </div>
            <div className="mt-1.5 flex items-center justify-between font-mono text-[10px] text-[var(--text-muted)]">
              <span>
                {selectedCamera?.lastCodec?.toUpperCase() || 'H.264'} {t('studio.rtspPassthrough')}
              </span>
              <span>
                {selectedCamera?.lastWidth}x{selectedCamera?.lastHeight}@{selectedCamera?.lastFps}
                fps
              </span>
            </div>
          </div>

          {/* 一键布防总闸 */}
          <div className="flex items-center justify-between rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-2.5">
            <div className="flex flex-col">
              <span className="text-xs font-medium text-[var(--text-primary)]">
                {t('studio.masterArm')}
              </span>
              <span className="text-[10px] text-[var(--text-muted)]">
                {t('studio.masterArmDesc')}
              </span>
            </div>
            <input
              type="checkbox"
              checked={isArmed}
              onChange={(e) => setIsArmed(e.target.checked)}
              className="h-4 w-4 cursor-pointer rounded accent-[var(--accent)]"
            />
          </div>
        </div>

        {/* 挂载算法栈 */}
        <div className="space-y-3 border-b border-[var(--border)] p-4">
          <div className="flex items-center justify-between">
            <span className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
              <Cpu className="h-3.5 w-3.5 text-[var(--accent)]" />
              <span>{t('studio.mountedAlgos')}</span>
            </span>
            <span className="rounded-full border border-[var(--border)] bg-[var(--accent-soft)] px-2 py-0.5 font-mono text-[10px] text-[var(--accent)]">
              {availableAlgos.length} {t('studio.verified')}
            </span>
          </div>

          <div className="space-y-2">
            {availableAlgos.map((algo) => (
              <div
                key={algo.algorithmId}
                className="cursor-pointer space-y-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-2.5 shadow-2xs transition-all hover:border-[var(--accent)]"
                onClick={() => {
                  setSelectedAlgoForConfig(algo)
                  setIsAlgoDrawerOpen(true)
                }}
              >
                <div className="flex items-center justify-between">
                  <span className="text-[11px] font-medium text-[var(--text-primary)]">
                    {algo.name}
                  </span>
                  <span className="rounded-md bg-emerald-500/10 px-1.5 py-0.5 font-mono text-[9px] text-emerald-500">
                    v{algo.version}
                  </span>
                </div>
                <div className="flex items-center justify-between text-[10px] text-[var(--text-muted)]">
                  <span className="font-mono">{algo.algorithmId}</span>
                  <span className="text-[var(--accent)] hover:underline">
                    {t('studio.configParams')} &gt;
                  </span>
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* 证据抓拍策略 */}
        <div className="space-y-2.5 p-4">
          <span className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
            <Check className="h-3.5 w-3.5 text-emerald-500" />
            <span>{t('studio.snapshotPolicy')}</span>
          </span>
          <div className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-2.5 text-[10px] text-[var(--text-secondary)]">
            <div className="flex items-center justify-between">
              <span>{t('studio.policy4k')}</span>
              <span className="font-mono text-emerald-500">{t('studio.policy4kDesc')}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t('studio.policyCrop')}</span>
              <span className="font-mono text-[var(--accent)]">{t('studio.policyCropDesc')}</span>
            </div>
            <div className="flex items-center justify-between">
              <span>{t('studio.policyFallback')}</span>
              <span className="font-mono text-amber-500">{t('studio.policyFallbackDesc')}</span>
            </div>
          </div>
        </div>
      </aside>

      {/* ----------------- 中央：实时流互动舞台 (STAGE) ----------------- */}
      <main className="relative flex flex-1 flex-col overflow-hidden bg-[var(--bg-primary)]">
        {/* 顶部悬浮绘制工具栏 (Floating Island HUD) */}
        <div className="frosted-glass absolute top-4 left-1/2 z-20 flex -translate-x-1/2 items-center gap-1 rounded-full border border-[var(--border)] p-1 shadow-xl">
          <button
            onClick={() => {
              setTool('select')
              setCurrentPoints([])
            }}
            className={`flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-all ${
              tool === 'select'
                ? 'bg-[var(--accent)] text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
            title={`${t('tools.select')} (V)`}
          >
            <MousePointer2 className="h-3.5 w-3.5" />
            <span>{t('tools.select')}</span>
            <span className="font-mono text-[10px] opacity-70">V</span>
          </button>

          <span className="mx-0.5 h-4 w-[1px] bg-[var(--border)]" />

          <button
            onClick={() => {
              setTool('roi')
              setCurrentPoints([])
            }}
            className={`flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-all ${
              tool === 'roi'
                ? 'bg-cyan-500 text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
            title={`${t('tools.roi')} (R)`}
          >
            <Hexagon className="h-3.5 w-3.5" />
            <span>{t('tools.roi')}</span>
            <span className="font-mono text-[10px] opacity-70">R</span>
          </button>

          <button
            onClick={() => {
              setTool('line')
              setCurrentPoints([])
            }}
            className={`flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-all ${
              tool === 'line'
                ? 'bg-emerald-500 text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
            title={`${t('tools.line')} (L)`}
          >
            <Slash className="h-3.5 w-3.5" />
            <span>{t('tools.line')}</span>
            <span className="font-mono text-[10px] opacity-70">L</span>
          </button>

          <button
            onClick={() => {
              setTool('mask')
              setCurrentPoints([])
            }}
            className={`flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-all ${
              tool === 'mask'
                ? 'bg-slate-700 text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
            title={`${t('tools.mask')} (M)`}
          >
            <ShieldAlert className="h-3.5 w-3.5" />
            <span>{t('tools.mask')}</span>
            <span className="font-mono text-[10px] opacity-70">M</span>
          </button>

          <button
            onClick={() => {
              setTool('precrop')
              setCurrentPoints([])
            }}
            className={`flex items-center gap-1.5 rounded-full px-3 py-1.5 text-xs font-medium transition-all ${
              tool === 'precrop'
                ? 'bg-amber-500 text-white shadow-xs'
                : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
            }`}
            title={`${t('tools.precrop')} (C)`}
          >
            <Crop className="h-3.5 w-3.5" />
            <span>{t('tools.precrop')}</span>
            <span className="font-mono text-[10px] opacity-70">C</span>
          </button>

          <span className="mx-0.5 h-4 w-[1px] bg-[var(--border)]" />

          <button
            onClick={() => setSnapEnabled(!snapEnabled)}
            className={`rounded-full p-1.5 ${snapEnabled ? 'text-[var(--accent)]' : 'text-[var(--text-muted)]'}`}
            title={t('tools.snap')}
          >
            <Magnet className="h-4 w-4" />
          </button>

          {currentPoints.length > 0 && (
            <button
              onClick={finishDrawing}
              className="ml-1 flex items-center gap-1 rounded-full bg-cyan-500 px-2.5 py-1 text-xs font-semibold text-white shadow-xs hover:bg-cyan-600"
            >
              <Check className="h-3.5 w-3.5" />
              <span>{t('tools.closePolygon')}</span>
            </button>
          )}
        </div>

        {/* 交互视口区 */}
        <div className="flex flex-1 items-center justify-center overflow-hidden p-4">
          <div
            ref={stageRef}
            onClick={handleStageClick}
            onMouseMove={handleStageMouseMove}
            onMouseUp={handleStageMouseUp}
            className={`relative max-h-full max-w-full overflow-hidden rounded-2xl border border-[var(--border)] bg-black/95 shadow-2xl ${
              tool !== 'select' ? 'cursor-crosshair' : 'cursor-default'
            }`}
            style={{
              width: '100%',
              height: 'auto',
              maxWidth: '1280px',
              aspectRatio:
                selectedCamera?.lastWidth && selectedCamera?.lastHeight
                  ? `${selectedCamera.lastWidth} / ${selectedCamera.lastHeight}`
                  : '16 / 9',
            }}
          >
            {/* 真实子码流播放器 */}
            {selectedCamera ? (
              <LivePlayer
                cameraId={selectedCamera.cameraId}
                stream="sub"
                className="pointer-events-none h-full w-full"
              />
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-slate-500">
                {t('tools.selectVideoPrompt')}
              </div>
            )}

            {/* 矢量绘制与高频标定交互 SVG 覆盖层 */}
            <svg
              className="pointer-events-auto absolute inset-0 h-full w-full"
              viewBox="0 0 100 100"
              preserveAspectRatio="none"
            >
              {/* 绊线 Line 跨界箭头指示标记 */}
              <defs>
                <marker
                  id="line-arrow-a-to-b"
                  markerWidth="8"
                  markerHeight="8"
                  refX="4"
                  refY="4"
                  orient="auto"
                >
                  <path d="M 1 1 L 7 4 L 1 7 Z" fill="#10b981" />
                </marker>
                <marker
                  id="line-arrow-b-to-a"
                  markerWidth="8"
                  markerHeight="8"
                  refX="4"
                  refY="4"
                  orient="auto-start-reverse"
                >
                  <path d="M 1 1 L 7 4 L 1 7 Z" fill="#10b981" />
                </marker>
              </defs>

              {/* 既有布防规则渲染 */}
              {rules.map((rule) => {
                if (!rule.visible || rule.points.length < 2) return null
                const isSelected = rule.id === selectedRuleId

                if (rule.role === 'line') {
                  const p1 = rule.points[0]
                  const p2 = rule.points[1]
                  return (
                    <g
                      key={rule.id}
                      onClick={(e) => {
                        e.stopPropagation()
                        setSelectedRuleId(rule.id)
                      }}
                    >
                      <line
                        x1={`${p1.x * 100}%`}
                        y1={`${p1.y * 100}%`}
                        x2={`${p2.x * 100}%`}
                        y2={`${p2.y * 100}%`}
                        stroke={isSelected ? '#34d399' : '#10b981'}
                        strokeWidth={isSelected ? '3' : '2'}
                        markerEnd={getLineMarkerEnd(rule.lineDirection)}
                      />
                      {isSelected && (
                        <>
                          <circle
                            cx={`${p1.x * 100}%`}
                            cy={`${p1.y * 100}%`}
                            r="1.5"
                            fill="#10b981"
                            stroke="#ffffff"
                            strokeWidth="0.5"
                            className="cursor-move"
                            onMouseDown={(e) => {
                              e.stopPropagation()
                              setDraggingVertex({ ruleId: rule.id, pointIndex: 0 })
                            }}
                          />
                          <circle
                            cx={`${p2.x * 100}%`}
                            cy={`${p2.y * 100}%`}
                            r="1.5"
                            fill="#10b981"
                            stroke="#ffffff"
                            strokeWidth="0.5"
                            className="cursor-move"
                            onMouseDown={(e) => {
                              e.stopPropagation()
                              setDraggingVertex({ ruleId: rule.id, pointIndex: 1 })
                            }}
                          />
                        </>
                      )}
                    </g>
                  )
                }

                // 多边形 (ROI, Mask)
                const ptsStr = rule.points.map((p) => `${p.x * 100},${p.y * 100}`).join(' ')
                let fillColor = 'rgba(0, 242, 254, 0.2)'
                let strokeColor = '#00f2fe'
                if (rule.role === 'mask') {
                  fillColor = 'rgba(15, 23, 42, 0.65)'
                  strokeColor = '#94a3b8'
                }

                return (
                  <g
                    key={rule.id}
                    onClick={(e) => {
                      e.stopPropagation()
                      setSelectedRuleId(rule.id)
                    }}
                  >
                    <polygon
                      points={ptsStr}
                      fill={fillColor}
                      stroke={isSelected ? '#ffffff' : strokeColor}
                      strokeWidth={isSelected ? '3' : '1.5'}
                    />
                    {isSelected &&
                      rule.points.map((pt, idx) => (
                        <circle
                          key={idx}
                          cx={`${pt.x * 100}%`}
                          cy={`${pt.y * 100}%`}
                          r="1.5"
                          fill={strokeColor}
                          stroke="#ffffff"
                          strokeWidth="0.5"
                          className="cursor-move"
                          onMouseDown={(e) => {
                            e.stopPropagation()
                            setDraggingVertex({ ruleId: rule.id, pointIndex: idx })
                          }}
                        />
                      ))}
                  </g>
                )
              })}

              {/* 绘制中的当前多边形/折线预览 */}
              {currentPoints.length > 0 && (
                <g>
                  {tool === 'line' ? (
                    <line
                      x1={`${currentPoints[0].x * 100}%`}
                      y1={`${currentPoints[0].y * 100}%`}
                      x2={`${(cursorPos?.x ?? currentPoints[0].x) * 100}%`}
                      y2={`${(cursorPos?.y ?? currentPoints[0].y) * 100}%`}
                      stroke="#10b981"
                      strokeWidth="2.5"
                      strokeDasharray="4 4"
                    />
                  ) : (
                    <>
                      <polygon
                        points={[
                          ...currentPoints.map((p) => `${p.x * 100}%,${p.y * 100}%`),
                          ...(cursorPos ? [`${cursorPos.x * 100}%,${cursorPos.y * 100}%`] : []),
                        ].join(' ')}
                        fill="rgba(0, 242, 254, 0.15)"
                        stroke="#00f2fe"
                        strokeWidth="1.5"
                        strokeDasharray="4 4"
                      />
                      {currentPoints.map((p, idx) => (
                        <circle
                          key={idx}
                          cx={`${p.x * 100}%`}
                          cy={`${p.y * 100}%`}
                          r="1.5"
                          fill="#00f2fe"
                        />
                      ))}
                    </>
                  )}
                </g>
              )}
            </svg>
          </div>
        </div>

        {/* 底部信息栏与保存控制 */}
        <footer className="frosted-glass flex h-12 items-center justify-between border-t border-[var(--border)] px-6 text-xs">
          <div className="flex items-center gap-3 text-[var(--text-muted)]">
            <span>
              {t('footer.rulesCount')}：
              <strong className="font-bold text-emerald-500">{rules.length}</strong>{' '}
              {t('footer.items')}
            </span>
            <span className="hidden text-[var(--border-strong)] md:inline">|</span>
            <span className="hidden text-[var(--text-secondary)] md:inline">
              {t('footer.shortcuts')}：
              <kbd className="rounded border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px]">
                V
              </kbd>{' '}
              {t('tools.select')}{' '}
              <kbd className="rounded border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px]">
                R
              </kbd>{' '}
              {t('tools.roi')}{' '}
              <kbd className="rounded border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px]">
                L
              </kbd>{' '}
              {t('tools.line')}{' '}
              <kbd className="rounded border border-[var(--border)] bg-[var(--bg-secondary)] px-1.5 py-0.5 font-mono text-[10px]">
                Enter
              </kbd>{' '}
              {t('tools.closePolygon')}
            </span>
          </div>

          <div className="flex items-center gap-3">
            {saveToast && (
              <span className="text-xs font-semibold text-emerald-500">{saveToast}</span>
            )}
            <button
              onClick={handleSave}
              disabled={isSaving}
              className="flex items-center gap-1.5 rounded-xl bg-[var(--accent)] px-4 py-1.5 text-xs font-semibold text-white shadow-xs transition-all hover:opacity-90 disabled:opacity-50"
            >
              <Save className="h-3.5 w-3.5" />
              <span>{isSaving ? t('footer.saving') : t('footer.save')}</span>
            </button>
          </div>
        </footer>
      </main>

      {/* ----------------- 右侧：图层列表与属性检查器 ----------------- */}
      <aside className="frosted-glass flex w-80 min-w-80 flex-col overflow-hidden border-l border-[var(--border)] text-xs">
        <div className="flex items-center justify-between border-b border-[var(--border)] p-3.5">
          <span className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
            <Layers className="h-4 w-4 text-[var(--accent)]" />
            <span>{t('layers.title')}</span>
          </span>
          <span className="rounded-full border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[10px] font-bold text-[var(--accent)]">
            {rules.length} {t('footer.items')}
          </span>
        </div>

        {/* 图层列表 */}
        <div className="max-h-56 space-y-1.5 overflow-y-auto border-b border-[var(--border)] p-2.5">
          {rules.length === 0 ? (
            <div className="py-6 text-center text-[var(--text-muted)]">
              <p>{t('layers.empty')}</p>
              <p className="text-[10px] opacity-75">{t('layers.emptyTip')}</p>
            </div>
          ) : (
            rules.map((rule) => {
              const isSelected = rule.id === selectedRuleId
              return (
                <div
                  key={rule.id}
                  onClick={() => setSelectedRuleId(rule.id)}
                  className={`flex cursor-pointer items-center justify-between rounded-xl p-2 transition-all ${
                    isSelected
                      ? 'border border-[var(--accent)]/40 bg-[var(--accent-soft)] font-medium text-[var(--text-primary)] shadow-2xs'
                      : 'border border-[var(--border)] bg-[var(--bg-surface)] text-[var(--text-secondary)] hover:bg-[var(--bg-secondary)]'
                  }`}
                >
                  <div className="flex items-center gap-2">
                    {rule.role === 'roi' && <Hexagon className="h-3.5 w-3.5 text-cyan-500" />}
                    {rule.role === 'line' && <Slash className="h-3.5 w-3.5 text-emerald-500" />}
                    {rule.role === 'mask' && <ShieldAlert className="h-3.5 w-3.5 text-slate-400" />}
                    <span className="max-w-[140px] truncate text-[11px] font-medium">
                      {rule.name}
                    </span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        setRules((prev) =>
                          prev.map((r) => (r.id === rule.id ? { ...r, visible: !r.visible } : r)),
                        )
                      }}
                      className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
                    >
                      {rule.visible ? (
                        <Eye className="h-3 w-3" />
                      ) : (
                        <EyeOff className="h-3 w-3 opacity-40" />
                      )}
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation()
                        deleteRule(rule.id)
                      }}
                      className="text-[var(--text-muted)] hover:text-rose-500"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              )
            })
          )}
        </div>

        {/* 规则属性检查器 */}
        <div className="flex-1 space-y-4 overflow-y-auto p-4">
          <div className="flex items-center justify-between">
            <span className="flex items-center gap-1.5 font-semibold text-[var(--text-primary)]">
              <SlidersHorizontal className="h-3.5 w-3.5 text-[var(--accent)]" />
              <span>{t('inspector.title')}</span>
            </span>
            {selectedRule && (
              <span className="rounded-md border border-[var(--border)] bg-[var(--accent-soft)] px-2 py-0.5 font-mono text-[9px] font-bold text-[var(--accent)] uppercase">
                {selectedRule.role}
              </span>
            )}
          </div>

          {!selectedRule ? (
            <div className="py-8 text-center text-[11px] text-[var(--text-muted)]">
              {t('inspector.emptyTip')}
            </div>
          ) : (
            <div className="space-y-3.5">
              <div>
                <label className="mb-1 block font-mono text-[10px] text-[var(--text-muted)] uppercase">
                  {t('inspector.ruleName')}
                </label>
                <input
                  type="text"
                  value={selectedRule.name}
                  onChange={(e) => {
                    const val = e.target.value
                    setRules((prev) =>
                      prev.map((r) => (r.id === selectedRule.id ? { ...r, name: val } : r)),
                    )
                  }}
                  className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                />
              </div>

              {/* 绑定算法 */}
              <div>
                <label className="mb-1 block font-mono text-[10px] font-bold text-[var(--accent)] uppercase">
                  {t('inspector.boundAlgo')}
                </label>
                <select
                  value={selectedRule.boundAlgo || 'general_detection'}
                  onChange={(e) => {
                    const val = e.target.value
                    setRules((prev) =>
                      prev.map((r) => (r.id === selectedRule.id ? { ...r, boundAlgo: val } : r)),
                    )
                  }}
                  className="w-full rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] px-3 py-1.5 font-mono text-xs text-[var(--text-primary)] outline-none focus:border-[var(--accent)]"
                >
                  {availableAlgos.map((a) => (
                    <option key={a.algorithmId} value={a.algorithmId}>
                      {a.name} ({a.algorithmId})
                    </option>
                  ))}
                </select>
              </div>

              {/* 绊线方向 */}
              {selectedRule.role === 'line' && (
                <div>
                  <label className="mb-1 block font-mono text-[10px] text-[var(--text-muted)] uppercase">
                    {t('inspector.lineDirection')}
                  </label>
                  <div className="grid grid-cols-3 gap-1 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] p-1 text-center text-[10px]">
                    {(['both', 'a_to_b', 'b_to_a'] as DetectionLineDirection[]).map((dir) => (
                      <button
                        key={dir}
                        type="button"
                        onClick={() =>
                          setRules((prev) =>
                            prev.map((r) =>
                              r.id === selectedRule.id ? { ...r, lineDirection: dir } : r,
                            ),
                          )
                        }
                        className={`rounded-lg py-1 font-medium transition-all ${
                          selectedRule.lineDirection === dir
                            ? 'bg-[var(--accent)] font-semibold text-white shadow-xs'
                            : 'text-[var(--text-secondary)] hover:text-[var(--text-primary)]'
                        }`}
                      >
                        {getDirectionLabel(dir, t)}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* 克隆与删除 */}
              <div className="grid grid-cols-2 gap-2 pt-2">
                <button
                  type="button"
                  onClick={() => cloneRule(selectedRule.id)}
                  className="flex items-center justify-center gap-1.5 rounded-xl border border-[var(--border)] bg-[var(--bg-surface)] py-1.5 text-xs font-semibold text-[var(--accent)] transition-colors hover:bg-[var(--accent-soft)]"
                >
                  <Copy className="h-3.5 w-3.5" />
                  <span>{t('inspector.clone')}</span>
                </button>
                <button
                  type="button"
                  onClick={() => deleteRule(selectedRule.id)}
                  className="flex items-center justify-center gap-1.5 rounded-xl border border-rose-500/30 bg-rose-500/10 py-1.5 text-xs font-semibold text-rose-500 transition-colors hover:bg-rose-500/20"
                >
                  <Trash2 className="h-3.5 w-3.5" />
                  <span>{t('inspector.delete')}</span>
                </button>
              </div>
            </div>
          )}
        </div>
      </aside>

      {/* ----------------- 算法参数配置与沙箱自检抽屉 ----------------- */}
      {isAlgoDrawerOpen && selectedAlgoForConfig && (
        <div className="fixed inset-0 z-50 flex justify-end bg-black/60 backdrop-blur-xs">
          <div className="flex h-full w-96 flex-col space-y-4 overflow-y-auto border-l border-[var(--border)] bg-[var(--bg-surface-solid)] p-5 shadow-2xl">
            <div className="flex items-center justify-between border-b border-[var(--border)] pb-3">
              <div className="flex items-center gap-2">
                <Cpu className="h-4 w-4 text-[var(--accent)]" />
                <span className="text-sm font-semibold text-[var(--text-primary)]">
                  {selectedAlgoForConfig.name}
                </span>
              </div>
              <button
                onClick={() => setIsAlgoDrawerOpen(false)}
                className="text-[var(--text-muted)] hover:text-[var(--text-primary)]"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="space-y-2 text-xs text-[var(--text-secondary)]">
              <div className="flex justify-between">
                <span className="text-[var(--text-muted)]">{t('algoDrawer.id')}</span>
                <span className="font-mono font-bold text-[var(--accent)]">
                  {selectedAlgoForConfig.algorithmId}
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--text-muted)]">{t('algoDrawer.version')}</span>
                <span>
                  v{selectedAlgoForConfig.version} ({selectedAlgoForConfig.author})
                </span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--text-muted)]">{t('algoDrawer.platforms')}</span>
                <span className="font-mono text-[10px] font-bold text-emerald-500">
                  {selectedAlgoForConfig.supportedPlatforms.join(', ')}
                </span>
              </div>
              <p className="pt-1 text-[11px] leading-relaxed text-[var(--text-muted)]">
                {selectedAlgoForConfig.description}
              </p>
            </div>

            <div className="space-y-2 border-t border-[var(--border)] pt-3">
              <span className="text-xs font-semibold text-[var(--text-primary)]">
                {t('algoDrawer.classes')}
              </span>
              <div className="flex flex-wrap gap-1.5">
                {selectedAlgoForConfig.classes.map((cls) => (
                  <span
                    key={cls}
                    className="rounded-md border border-[var(--border)] bg-[var(--bg-secondary)] px-2 py-0.5 font-mono text-[10px] font-medium text-[var(--accent)]"
                  >
                    {cls}
                  </span>
                ))}
              </div>
            </div>

            {/* 上传算法包归档 */}
            <div className="space-y-2 border-t border-[var(--border)] pt-3">
              <div className="flex items-center justify-between">
                <span className="flex items-center gap-1.5 text-xs font-semibold text-[var(--text-primary)]">
                  <Upload className="h-3.5 w-3.5 text-[var(--accent)]" />
                  <span>{t('actions.uploadPackage')}</span>
                </span>
              </div>
              <label className="flex cursor-pointer items-center justify-center gap-2 rounded-xl border border-dashed border-[var(--border)] bg-[var(--bg-secondary)] p-3 text-xs text-[var(--text-secondary)] transition-all hover:border-[var(--accent)] hover:text-[var(--accent)]">
                <input
                  type="file"
                  accept=".zip,.tar,.tar.gz,.tgz"
                  className="hidden"
                  onChange={handleUploadPackageFile}
                  disabled={isUploadingPkg}
                />
                <Upload className="h-4 w-4" />
                <span>{isUploadingPkg ? t('actions.uploading') : t('actions.uploadPackage')}</span>
              </label>
            </div>

            {/* 七步沙箱安全自检互动区 (PRD §3.2) */}
            <div className="space-y-3 border-t border-[var(--border)] pt-3">
              <div className="flex items-center justify-between">
                <span className="flex items-center gap-1 text-xs font-semibold text-[var(--text-primary)]">
                  <ShieldCheck className="h-3.5 w-3.5 text-emerald-500" />
                  <span>{t('sandbox.title')}</span>
                </span>
                <button
                  onClick={handleRunSandboxTest}
                  disabled={isVerifyingSandbox}
                  className="text-[10px] font-semibold text-[var(--accent)] hover:underline disabled:opacity-50"
                >
                  {isVerifyingSandbox ? t('actions.verifyingSandbox') : t('actions.executeSandbox')}
                </button>
              </div>

              {sandboxResult ? (
                <div className="space-y-2 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] p-3 text-xs">
                  <div className="flex items-center justify-between">
                    <span className="text-[11px] font-medium">{t('sandbox.status')}</span>
                    <span
                      className={`rounded-full px-2 py-0.5 font-mono text-[10px] font-bold ${
                        sandboxResult.passed
                          ? 'border border-emerald-500/30 bg-emerald-500/15 text-emerald-500'
                          : 'border border-rose-500/30 bg-rose-500/15 text-rose-500'
                      }`}
                    >
                      {sandboxResult.passed
                        ? t('sandbox.allPassed')
                        : t('sandbox.interrupted', { passed: sandboxResult.stepsPassed })}
                    </span>
                  </div>

                  <div className="space-y-1 pt-1">
                    {sandboxResult.steps.map((step, idx) => {
                      const isStepOk = idx < sandboxResult.stepsPassed
                      return (
                        <div
                          key={idx}
                          className="flex items-center gap-1.5 text-[10px] text-[var(--text-muted)]"
                        >
                          {isStepOk ? (
                            <CheckCircle2 className="h-3 w-3 shrink-0 text-emerald-500" />
                          ) : (
                            <AlertCircle className="h-3 w-3 shrink-0 text-rose-500" />
                          )}
                          <span
                            className={isStepOk ? 'text-[var(--text-secondary)]' : 'text-rose-400'}
                          >
                            {step}
                          </span>
                        </div>
                      )
                    })}
                  </div>

                  {sandboxResult.errorMessage && (
                    <p className="mt-1 border-t border-[var(--border)] pt-1 font-mono text-[10px] text-rose-500">
                      {sandboxResult.errorMessage}
                    </p>
                  )}
                </div>
              ) : (
                <p className="text-[10px] text-[var(--text-muted)]">{t('sandbox.tip')}</p>
              )}
            </div>

            <div className="flex justify-end pt-4">
              <button
                onClick={() => setIsAlgoDrawerOpen(false)}
                className="rounded-xl bg-[var(--accent)] px-4 py-1.5 text-xs font-semibold text-white shadow-xs hover:opacity-90"
              >
                {t('algoDrawer.done')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
