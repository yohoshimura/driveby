// OWASP DevSecOps Guideline 2-2-1-3 (linting) and 2-3-1-1 (SAST): catches
// the XSS and code-injection sinks that matter in a webview allowed to call
// Rust commands. CI fails on errors; warnings are reported until burned down.
import js from '@eslint/js';
import globals from 'globals';
import eslintReact from '@eslint-react/eslint-plugin';
import reactHooks from 'eslint-plugin-react-hooks';
import security from 'eslint-plugin-security';

export default [
  { ignores: ['dist/**', 'src-tauri/**', 'node_modules/**'] },
  js.configs.recommended,
  security.configs.recommended,
  { files: ['**/*.{js,jsx}'], ...eslintReact.configs.recommended },
  // src/ already carries `eslint-disable ... react-hooks/exhaustive-deps`
  // comments, so the hooks rules come from eslint-plugin-react-hooks and
  // @eslint-react's duplicates are switched off.
  { files: ['**/*.{js,jsx}'], ...eslintReact.configs['disable-conflict-eslint-plugin-react-hooks'] },
  {
    files: ['**/*.{js,jsx}'],
    plugins: { 'react-hooks': reactHooks },
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: { ...globals.browser, __APP_VERSION__: 'readonly' },
      parserOptions: { ecmaFeatures: { jsx: true } },
    },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      '@eslint-react/exhaustive-deps': 'off',
      'no-unused-vars': ['error', { varsIgnorePattern: '^(React$|_)', argsIgnorePattern: '^_', ignoreRestSiblings: true }],
      'no-eval': 'error',
      'no-implied-eval': 'error',
      'no-new-func': 'error',
      'no-script-url': 'error',
      // XSS surface in a webview that can call Rust commands: errors, not warnings.
      '@eslint-react/dom-no-dangerously-set-innerhtml': 'error',
      '@eslint-react/dom-no-script-url': 'error',
      '@eslint-react/dom-no-unsafe-target-blank': 'error',
      '@eslint-react/dom-no-unsafe-iframe-sandbox': 'error',
      // >80% false positives on plain object/array indexing (OWASP 2-3-1-1).
      'security/detect-object-injection': 'off',
      'no-empty': ['error', { allowEmptyCatch: true }],
      'no-irregular-whitespace': ['error', { skipRegExps: true }],
    },
  },
  // isValidFolderName rejects \x00-\x1f exactly as Windows does.
  { files: ['src/lib/task.js'], rules: { 'no-control-regex': 'off' } },
  { files: ['**/__tests__/**', '**/*.test.{js,jsx}', 'vite.config.js', 'scripts/**/*.mjs'], languageOptions: { globals: { ...globals.node } } },
];
