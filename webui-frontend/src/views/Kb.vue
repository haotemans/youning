<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">知识库</span>
      </div>
      <n-button size="small" secondary @click="load">刷新</n-button>
    </div>

    <n-grid cols="1 s:2" :x-gap="16" :y-gap="16" responsive="screen" style="max-width: 640px">
      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" :style="{ background: s.enabled ? 'var(--yt-ok)' : 'var(--yt-ink-3)' }" />
            引擎状态
          </div>
          <div class="stat-foot">
            <span class="stat-num">{{ s.enabled ? '已启用' : '未启用' }}</span>
          </div>
        </n-card>
      </n-gi>
      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-primary)" />
            文档索引数
          </div>
          <div class="stat-foot">
            <span class="stat-num mono">{{ s.docs ?? 0 }}</span>
          </div>
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'

const s = ref({ enabled: false, docs: 0 })
async function load() {
  const { data } = await api.get('/kb/status')
  s.value = data
}
onMounted(load)
</script>
