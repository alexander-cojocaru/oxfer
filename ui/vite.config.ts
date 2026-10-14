import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { svelteTesting } from '@testing-library/svelte/vite';

import { version } from './package.json';

export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(version) },
  plugins: [svelte(), svelteTesting()],
  // Tauri serves the built files from a custom protocol; keep paths relative.
  base: './',
  clearScreen: false,
  // CHANGELOG.md lives at the repository root, outside the Vite root.
  server: { port: 5173, strictPort: true, fs: { allow: ['..'] } },
  build: {
    target: 'es2022',
    outDir: 'dist',
    emptyOutDir: true,
    // No inline scripts/data URIs for scripts: the CSP only allows 'self'.
    assetsInlineLimit: 0,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.ts'],
  },
});
