import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Dev: Vite serves the UI and proxies /api to the Rust backend.
// Prod: nginx serves the built files and proxies /api itself.
const proxyTarget = process.env.VITE_PROXY_TARGET || 'http://127.0.0.1:8791';

export default defineConfig({
  plugins: [svelte()],
  // Svelte ships a server build under the default (node) condition; tests mount
  // components in jsdom, so resolution has to pick the browser build.
  resolve: {
    conditions: ['browser', 'module', 'import', 'default'],
  },
  server: {
    port: 8790,
    strictPort: true,
    proxy: {
      '/api': { target: proxyTarget, changeOrigin: true },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.js'],
  },
});
