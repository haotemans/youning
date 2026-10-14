<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">记忆库</span>
        <span class="yt-page-sub mono">{{ memories.length }} 条记忆 · {{ summaries.length }} 条摘要</span>
      </div>
      <n-button size="small" secondary :loading="loading" @click="load">刷新</n-button>
    </div>

    <div class="yt-toolbar">
      <n-select v-model:value="ownerType" :options="typeOptions" clearable placeholder="归属类型" size="small" style="width: 150px" @update:value="load" />
      <n-input v-model:value="ownerId" placeholder="归属 ID（如 p_2001）" size="small" style="width: 220px" @keyup.enter="load" />
      <n-button size="small" type="primary" secondary @click="load">查询</n-button>
    </div>

    <n-card size="small">
      <n-tabs type="line" animated>
        <n-tab-pane name="long" :tab="`长期记忆 (${memories.length})`">
          <n-skeleton v-if="!firstLoaded" text :repeat="6" style="margin-top: 8px" />
          <n-data-table v-else :columns="memCols" :data="memories" size="small" :loading="loading" :pagination="{ pageSize: 15 }" />
        </n-tab-pane>
        <n-tab-pane name="daily" :tab="`每日摘要 (${summaries.length})`">
          <n-skeleton v-if="!firstLoaded" text :repeat="6" style="margin-top: 8px" />
          <n-data-table v-else :columns="sumCols" :data="summaries" size="small" :loading="loading" :pagination="{ pageSize: 15 }" />
        </n-tab-pane>
      </n-tabs>
    </n-card>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'
import { fmtTs } from '../fmt'

const ownerType = ref(null)
const ownerId = ref('')
/** @type {import('vue').Ref<any[]>} */
const memories = ref([])
/** @type {import('vue').Ref<any[]>} */
const summaries = ref([])
const loading = ref(false)
const firstLoaded = ref(false)
const typeOptions = ['person', 'chat', 'self'].map((s) => ({ label: s, value: s }))

const memCols = [
  { title: '归属对象', key: 'owner', width: 180, render: (r) => `${r.owner_type} / ${r.owner_id}` },
  { title: '记忆内容', key: 'content', ellipsis: { tooltip: true } },
  { title: '来源', key: 'source', width: 120 },
  { title: '更新时间', key: 'updated_at', width: 170, render: (r) => fmtTs(r.updated_at) },
]
const sumCols = [
  { title: '日期', key: 'date', width: 120 },
  { title: '归属对象', key: 'owner', width: 180, render: (r) => `${r.owner_type} / ${r.owner_id}` },
  { title: '摘要内容', key: 'summary', ellipsis: { tooltip: true } },
  { title: '消息区间', key: 'range', width: 140, render: (r) => (r.msg_id_end ? `${r.msg_id_start}~${r.msg_id_end}` : '—') },
]

async function load() {
  loading.value = true
  try {
    const p = new URLSearchParams({ limit: '200' })
    if (ownerType.value) p.set('owner_type', ownerType.value)
    if (ownerId.value) p.set('owner_id', ownerId.value)
    const [m, s] = await Promise.all([
      api.get(`/memories?${p}`), api.get(`/summaries?${p}`),
    ])
    memories.value = m.data.memories || []
    summaries.value = s.data.summaries || []
  } finally {
    loading.value = false
    firstLoaded.value = true
  }
}
onMounted(load)
</script>
