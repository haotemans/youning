<template>
  <div ref="container" class="meme-image">
    <n-image v-if="src && !failed" :src="src" :alt="alt" :img-props="{ style: 'width: 100%; height: 148px; object-fit: contain' }" @error="imageFailed" />
    <div v-else class="image-placeholder">
      <template v-if="failed">
        <span>图片加载失败</span>
        <n-button size="tiny" secondary @click="load">重试图片</n-button>
      </template>
      <span v-else>{{ loading ? '加载图片…' : '等待加载' }}</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api } from '../api'

const props = defineProps<{ id: number; alt: string }>()
const container = ref<HTMLElement | null>(null)
const src = ref('')
const failed = ref(false)
const loading = ref(false)
let observer: IntersectionObserver | undefined
let controller: AbortController | undefined
let visible = false

function release() {
  controller?.abort()
  if (src.value) URL.revokeObjectURL(src.value)
  src.value = ''
}
async function load() {
  release()
  const request = new AbortController()
  controller = request
  loading.value = true
  failed.value = false
  try {
    const { data } = await api.get(`/meme-file/${props.id}`, { responseType: 'blob', signal: request.signal })
    if (!request.signal.aborted) src.value = URL.createObjectURL(data)
  } catch {
    if (!request.signal.aborted) failed.value = true
  } finally {
    if (controller === request) loading.value = false
  }
}
function imageFailed() {
  release()
  failed.value = true
}
onMounted(() => {
  observer = new IntersectionObserver(entries => {
    if (entries.some(entry => entry.isIntersecting)) {
      visible = true
      observer?.disconnect()
      load()
    }
  }, { rootMargin: '200px' })
  if (container.value) observer.observe(container.value)
})
watch(() => props.id, () => { release(); if (visible) load() })
onBeforeUnmount(() => { observer?.disconnect(); release() })
</script>

<style scoped>
.meme-image { width: 100%; min-height: 148px; display: flex; justify-content: center; align-items: center; }
.meme-image :deep(.n-image) { width: 100%; cursor: zoom-in; }
.image-placeholder { min-height: 148px; display: flex; flex-direction: column; gap: 10px; justify-content: center; align-items: center; font-size: 12px; color: var(--yt-ink-3); }
</style>
