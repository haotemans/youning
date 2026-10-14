import fs from 'node:fs'
import path from 'node:path'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// 构建前清理旧的 static/assets 哈希产物，同时保留 static/legacy-index.html 兜底页
function cleanStaticAssets() {
  return {
    name: 'clean-static-assets',
    buildStart() {
      const assetsDir = path.resolve(process.cwd(), '../crates/webui/static/assets')
      if (fs.existsSync(assetsDir)) {
        fs.rmSync(assetsDir, { recursive: true, force: true })
      }
    },
  }
}

export default defineConfig({
  plugins: [vue(), cleanStaticAssets()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:8085',
      '/healthz': 'http://127.0.0.1:8085',
      '/ws': { target: 'ws://127.0.0.1:8085', ws: true },
    },
  },
  build: {
    outDir: '../crates/webui/static',
    emptyOutDir: false,
    chunkSizeWarningLimit: 2500,
  },
})
