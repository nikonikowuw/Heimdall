import js from '@eslint/js'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import globals from 'globals'
import tseslint from 'typescript-eslint'

/**
 * 硬编码具名色的判定正则（不含 esquery 的属性包裹）。
 *
 * 用正则而非枚举类名：新增 Tailwind 色阶时无需改配置就能拦住。
 * 依据 docs/nuwa/frontend/styling-guidelines.md#主题与材质。
 */
const NAMED_COLOUR =
  '\\b(?:bg|text|border|from|to|via|ring|fill|stroke|divide|outline|caret|decoration|placeholder|accent|shadow|outline-offset)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\\d{2,3}\\b'

export default tseslint.config(
  { ignores: ['dist', 'node_modules'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      /*
       * React Compiler 1.0 的编译期规则集（react-hooks v7 起并入 hooks 插件）。
       *
       * 用 `recommended-latest`：在 `recommended` 基础上多一条 `void-use-memo`。
       * 关键规则与本地约定：
       * - `refs`：禁止渲染期读写 ref。仓库统一用 `hooks/use-latest-ref.ts`
       *   把「最新值快照」交给 effect 同步，不要在组件里写 `ref.current = value`。
       * - `set-state-in-effect`：effect 内同步 setState 会多一轮渲染。
       *   取数类改「查询签名派生 + `.then` 链」，
       *   重置类改「渲染期状态调整」，难以拆解的用微任务包裹，
       *   详见 docs/nuwa/frontend/hook-guidelines.md#effect-内不要同步-setstate。
       * - `purity`/`immutability`/`globals`：渲染期禁调不纯函数（如 `Date.now()`）。
       *   实测：`async` + `try/finally` 会被判为同步 setState，等价 `.then` 链不会；
       *   effect 内写 ref 允许，渲染期读写 ref 报错。
       * - `preserve-manual-memoization`：已有 memo 的依赖必须与其推断一致，
       *   否则整个组件被编译器跳过优化。
       *
       * 注意：逐帧数据（检测框/遥测/PTS）不走 React 渲染，编译器管不到，
       * 仍必须保持 ref/Canvas/外部 store，见同文档「逐帧路径不走编译器」。
       *
       * 本次仅升级插件与规则，未启用 React Compiler 本体（babel-plugin-react-compiler）；
       * 启用前应先让这些规则全绿，并测量编译后包体与运行时收益。
       */
      ...reactHooks.configs['recommended-latest'].rules,
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'error',
      '@typescript-eslint/no-explicit-any': 'error',
      // 模块边界：跨目录引用一律走 @/ 别名，禁止多级相对路径。
      // 同域 ./ 与单层 ../（如 features/alarms/components → ../utils）仍然允许。
      // 依据 docs/nuwa/frontend/directory-structure.md#模块边界。
      '@typescript-eslint/no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['../../**', '../../../**', '../../../../**'],
              message:
                '跨目录请使用 @/ 别名，禁止 ../../ 及更深的相对路径（规范: docs/nuwa/frontend/directory-structure.md#模块边界）',
            },
          ],
        },
      ],
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
      // 补 no-restricted-imports 的缺口：它只看 ImportDeclaration，
      // 不覆盖 import('...') 动态导入与类型位置的 import('...').T。
      'no-restricted-syntax': [
        'error',
        {
          selector: 'ImportExpression[source.value=/^\\.\\.\\/\\.\\.\\//]',
          message:
            '跨目录动态 import() 请使用 @/ 别名，禁止 ../../ 及更深的相对路径（规范: docs/nuwa/frontend/directory-structure.md#模块边界）',
        },
        {
          selector: 'TSImportType[source.value=/^\\.\\.\\/\\.\\.\\//]',
          message:
            '跨目录类型 import() 请使用 @/ 别名，禁止 ../../ 及更深的相对路径（规范: docs/nuwa/frontend/directory-structure.md#模块边界）',
        },
        // 硬编码具名色：一律改用语义 token / utility。同一正则在两处使用，
        // 因为 no-restricted-syntax 的选择器按 AST 节点类型区分（字面量 vs 模板字符串）。
        {
          selector: `Literal[value=/${NAMED_COLOUR}/]`,
          message:
            '禁止硬编码具名色（如 bg-emerald-500 / text-cyan-400）。状态语义用 text-status-success / bg-status-warning/10 / border-status-info 等语义 utility；中性色用 --text-* / --border 等主题 token。依据 docs/nuwa/frontend/styling-guidelines.md#主题与材质',
        },
        {
          selector: `TemplateElement[value.raw=/${NAMED_COLOUR}/]`,
          message:
            '禁止硬编码具名色（含模板字符串与条件类名）。状态语义用语义 utility，中性色用主题 token。依据 docs/nuwa/frontend/styling-guidelines.md#主题与材质',
        },
      ],
    },
  },
  /*
   * 矢量/分类色板：规范显式豁免的例外。
   * ROI 描边 / 绊线 / 遮罩的 color theme 需要在同一容器内并列区分多个规则，
   * 这些颜色不表达状态语义，套用语义 token 会让所有规则变成同一颜色、丧失可区分性。
   * 依据 docs/nuwa/frontend/styling-guidelines.md#主题与材质「例外：矢量/分类色板」。
   *
   * 置后声明：flat config 后者覆盖前者，且本块须保留主配置已解析的 parser 与 plugins。
   */
  {
    files: ['src/features/tasks/components/rulesStudioTypes.ts'],
    rules: { 'no-restricted-syntax': 'off' },
  },
)
