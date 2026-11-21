import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// Tauri 需要固定端口与严格端口（端口被占用时直接失败，而不是自动换端口）
export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: {
    target: 'chrome110',
    minify: 'esbuild',
    sourcemap: false,
  },
})
