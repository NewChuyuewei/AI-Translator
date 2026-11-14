import { defineConfig } from 'vite';
import { resolve } from 'node:path';
export default defineConfig({
  root: 'src',
  clearScreen: false,
  build: {
    outDir: '../dist', emptyOutDir: true,
    rollupOptions: { input: { main: resolve('src/index.html'), control: resolve('src/control.html') } }
  },
  server: { port: 1420, strictPort: true, host: '0.0.0.0', allowedHosts: true }
});
