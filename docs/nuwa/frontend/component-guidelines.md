# 组件规范

## 接口与归属

- 使用具名函数导出，不用 `export default` / `React.FC`；`<ComponentName>Props` 放在组件附近，默认值在参数解构中设置。
- 展示组件接收数据和回调，请求交给资源 hook/页面容器与统一 API client。
- 动态列表 key 使用稳定业务 ID；高频事件列表项用 `memo`，回调保持稳定。
- 复用现有 UI 与主题样式；所有可见文本遵循 [i18n](./directory-structure.md#国际化)。

## 实时页面

- 播放器用 `React.memo` 隔离，camera/source/回调等 props 保持稳定，无关事件不重建播放器。
- 15～30fps 检测元数据放 `useRef`/Worker，通过 `requestAnimationFrame` 绘制透明 Canvas；不使用 React state 驱动高频 DOM。
- Zustand 按字段订阅，见 [状态规范](./state-management.md)；事件缓冲有界。
- 常态监控保持低噪，违规时通过稀疏事件显示轻量告警卡片与声音，不持续展示无违规噪点框。
- ROI / Mask / Line 编辑叠加在动态子码流上，坐标规范见 [全局约定](../guides/conventions.md#坐标)。

## 播放器生命周期

实现入口：[LivePlayer](../../../web/src/features/live/components/LivePlayer.tsx)、[WebCodecs 能力与协议](../../../web/src/lib/webcodecs.ts)。

- HTTP-FLV/mpegts.js 支持 H.264 与 Enhanced FLV H.265（`hvc1`）；实际硬解能力通过浏览器检测，不承诺所有浏览器都支持。
- FLV 低延迟路径启用 `liveBufferLatencyChasing` 和 `enableWorker`；100～200ms 是验证目标，不能代替实测。
- 辅轨拉子码流，主屏默认主码流并支持切流；这不改变后端仅对子码流常驻推理的分工。
- 卸载、换摄像头或换协议时销毁旧播放器、SourceBuffer、Worker、订阅、定时器与 RAF；MSE 实例必须调用 `destroy()`。
- 断流展示可恢复状态，重试规则见 [错误处理](./error-handling.md#连接恢复)。

## 交互

- 控件使用语义化 button/link，图标按钮有翻译后的 `aria-label`，表单关联 label，保持可见焦点。
- 面板沿用 `.frosted-glass`，自定义光标目标加 `.reticle-target`；生成 UI 通过封装定制。
- 浮层按栈响应 ESC，避免穿透关闭外层；表单/确认弹窗支持 Enter。
- 高风险媒体/图形视口局部错误隔离，不牵连导航和其他监控路。

验证无关事件下播放器稳定、动态 key、键盘行为及反复挂载/切流后的资源释放。
