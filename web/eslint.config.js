import js from '@eslint/js'
import reactHooks from 'eslint-plugin-react-hooks'
import reactRefresh from 'eslint-plugin-react-refresh'
import globals from 'globals'
import tseslint from 'typescript-eslint'

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
      ...reactHooks.configs.recommended.rules,
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
      ],
    },
  },
)
