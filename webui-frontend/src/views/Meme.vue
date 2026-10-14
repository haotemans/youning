<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">表情图库</span>
        <span class="yt-page-sub mono">待审 {{ pending.length }} · 已入库 {{ active.length }}</span>
      </div>
      <n-button size="small" secondary @click="load">刷新</n-button>
    </div>

    <n-card size="small">
      <n-tabs type="line" animated @update:value="load">
        <n-tab-pane name="pending" :tab="`待审队列 (${pending.length})`">
          <empty-state v-if="!pending.length" title="暂无待审表情包" />
          <div v-else class="meme-grid">
            <div v-for="m in pending" :key="m.id" class="meme-card">
              <div class="meme-preview">
                <MemeImg :id="m.id" height="120px" />
              </div>
              <n-input v-model:value="m._cat" size="small" placeholder="分类标签（可选）" />
              <div class="meme-actions">
                <n-button size="small" type="primary" secondary style="flex: 1" @click="approve(m)">入库</n-button>
                <n-popconfirm @positive-click="reject(m)">
                  <template #trigger>
                    <n-button size="small" type="error" quaternary>删除</n-button>
                  </template>
                  确认删除该表情文件？
                </n-popconfirm>
              </div>
            </div>
          </div>
        </n-tab-pane>

        <n-tab-pane name="active" :tab="`已入库 (${active.length})`">
          <div class="meme-filter" v-if="active.length">
            <n-select v-model:value="cat" :options="catOptions" clearable placeholder="按分类筛选" size="small" style="width: 180px" />
          </div>
          <empty-state v-if="!shownActive.length" title="暂无入库表情包" />
          <div v-else class="meme-grid">
            <div v-for="m in shownActive" :key="m.id" class="meme-card">
              <div class="meme-preview">
                <MemeImg :id="m.id" height="120px" />
              </div>
              <div class="meme-meta">
                <n-tag size="tiny" :bordered="false" type="info">{{ m.category || '默认' }}</n-tag>
                <span class="mono meme-count">{{ m.use_count }} 次</span>
              </div>
            </div>
          </div>
        </n-tab-pane>
      </n-tabs>
    </n-card>
  </div>
</template>

<script setup>
import { computed, h, onMounted, ref } from 'vue'
import { NImage } from 'naive-ui'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'

const MemeImg = (props) => {
  const src = ref('')
  api.get(`/meme-file/${props.id}`, { responseType: 'blob' }).then((r) => {
    src.value = URL.createObjectURL(r.data)
  })
  return () => h(NImage, { src: src.value, style: `height:${props.height}; object-fit: contain; width: 100%`, previewDisabled: !src.value })
}

const pending = ref([])
const active = ref([])
const cat = ref(null)

const catOptions = computed(() => [...new Set(active.value.map((m) => m.category).filter(Boolean))].map((c) => ({ label: c, value: c })))
const shownActive = computed(() => (cat.value ? active.value.filter((m) => m.category === cat.value) : active.value))

async function load() {
  const [p, a] = await Promise.all([
    api.get('/memes?status=pending'), api.get('/memes?status=active'),
  ])
  pending.value = (p.data.memes || []).map((m) => ({ ...m, _cat: '' }))
  active.value = a.data.memes || []
}
async function approve(m) {
  await api.post(`/memes/${m.id}/approve`, { category: m._cat || undefined })
  await load()
}
async function reject(m) {
  await api.post(`/memes/${m.id}/reject`)
  await load()
}
onMounted(load)
</script>

<style scoped>
.meme-filter {
  margin-bottom: 14px;
}
.meme-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 14px;
  margin-top: 8px;
}
.meme-card {
  border: 1px solid var(--yt-card-border);
  border-radius: 10px;
  background: var(--yt-elevated);
  padding: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.meme-preview {
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  padding: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 132px;
}
.meme-actions {
  display: flex;
  gap: 8px;
  align-items: center;
}
.meme-meta {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.meme-count {
  font-size: 12px;
  color: var(--yt-ink-3);
}
</style>
