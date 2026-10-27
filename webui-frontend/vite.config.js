import fs from 'node:fs'
import path from 'node:path'
import { gzipSync } from 'node:zlib'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// 构建前清理旧的 static/assets 哈希产物，同时保留 static/legacy-index.html 兜底页
/** @returns {import('vite').Plugin} */
function cleanStaticAssets() {
  return {
    name: 'clean-static-assets',
    apply: 'build',
    buildStart() {
      const assetsDir = path.resolve(process.cwd(), '../crates/webui/static/assets')
      if (fs.existsSync(assetsDir)) {
        fs.rmSync(assetsDir, { recursive: true, force: true })
      }
    },
  }
}

/** @returns {import('vite').Plugin} */
function precompressStaticAssets() {
  return {
    name: 'precompress-static-assets',
    apply: 'build',
    closeBundle() {
      const root = path.resolve(process.cwd(), '../crates/webui/static')
      for (const name of fs.readdirSync(root, { recursive: true, encoding: 'utf8' })) {
        if (!/\.(?:js|css|html|svg|json)$/.test(name)) continue
        const file = path.join(root, name)
        if (!fs.statSync(file).isFile()) continue
        const source = fs.readFileSync(file)
        fs.rmSync(`${file}.gz`, { force: true })
        if (source.length < 1024) continue
        const compressed = gzipSync(source, { level: 9 })
        if (compressed.length < source.length) fs.writeFileSync(`${file}.gz`, compressed)
      }
    },
  }
}

export default defineConfig({
  plugins: [vue(), cleanStaticAssets(), precompressStaticAssets()],
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
