import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import pkg from './package.json';

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Single source for the human-facing version string (Sidebar, Settings).
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // Don't watch the Rust source/build output — cargo locks .exe files
      // during compilation, which makes Node's fs watcher throw EBUSY.
      ignored: ['**/src-tauri/**'],
    },
  },
  // Only these prefixes reach import.meta.env. A bare 'TAURI_' also matched
  // TAURI_SIGNING_PRIVATE_KEY(_PASSWORD), which `tauri build` passes on to
  // beforeBuildCommand: any `import.meta.env` object access would inline the
  // updater signing key into the shipped bundle (the CVE-2023-46115 pattern).
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    target: 'es2021',
    minify: 'esbuild',
    sourcemap: false,
  },
});
