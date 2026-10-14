<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">任务回放</span>
        <span class="yt-page-sub mono">{{ list.length }} 个任务</span>
      </div>
      <n-button size="small" secondary @click="load">刷新</n-button>
    </div>

    <div class="yt-toolbar">
      <n-select v-model:value="state" :options="stateOptions" clearable placeholder="任务状态" size="small" style="width: 160px" @update:value="load" />
    </div>

    <n-grid cols="1 l:2" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card title="任务列表" size="small">
          <n-skeleton v-if="!firstLoaded" text :repeat="6" />
          <div v-else-if="list.length" class="task-list">
            <div v-for="t in list" :key="t.task_id" class="task-row" :class="{ active: cur?.task_id === t.task_id }" @click="open(t)">
              <n-tag size="tiny" :type="stateTag(t.state)" :bordered="false">{{ t.state }}</n-tag>
              <span class="task-goal">{{ t.goal }}</span>
              <span class="task-budget mono">{{ t.used_calls }}/{{ t.budget_max_calls }}</span>
            </div>
          </div>
          <empty-state v-else title="暂无任务记录" />
        </n-card>
      </n-gi>

      <n-gi>
        <n-card :title="cur ? `${cur.task_id} · ${cur.goal}` : '执行时间轴'" size="small">
          <template v-if="cur">
            <n-timeline v-if="events.length">
              <n-timeline-item v-for="e in events" :key="e.id" :title="`#${e.seq} ${e.kind}`" :content="textOf(e)" :time="fmtTs(e.ts)" />
            </n-timeline>
            <empty-state v-else title="暂无执行事件" />
          </template>
          <empty-state v-else title="选择左侧任务查看执行流水" />
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'
import { fmtTs } from '../fmt'
import EmptyState from '../components/EmptyState.vue'

const list = ref([])
const cur = ref(null)
const events = ref([])
const state = ref(null)
const firstLoaded = ref(false)
const stateOptions = ['pending', 'running', 'finished', 'failed', 'archived'].map((s) => ({ label: s, value: s }))

function stateTag(s) {
  return ({ running: 'success', finished: 'info', failed: 'error', archived: 'default', pending: 'warning' })[s] || 'default'
}
function textOf(e) {
  if (e.payload == null) return ''
  return typeof e.payload === 'string' ? e.payload : JSON.stringify(e.payload)
}

async function load() {
  const params = new URLSearchParams({ limit: '100' })
  if (state.value) params.set('state', state.value)
  const { data } = await api.get(`/tasks?${params}`)
  list.value = data.tasks || []
  firstLoaded.value = true
  if (list.value.length && !cur.value) open(list.value[0])
}
async function open(t) {
  cur.value = t
  const { data } = await api.get(`/tasks/${t.task_id}/events`)
  events.value = data.events || []
}
onMounted(load)
</script>

<style scoped>
.task-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: calc(100vh - 260px);
  overflow-y: auto;
}
.task-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.14s ease, border-color 0.14s ease;
}
.task-row:hover {
  background: var(--yt-elevated);
}
.task-row.active {
  background: var(--yt-active-bg);
  border-color: var(--yt-primary);
}
.task-goal {
  flex: 1;
  min-width: 0;
  font-size: 13.5px;
  color: var(--yt-ink-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.task-budget {
  font-size: 12px;
  color: var(--yt-ink-3);
}
</style>
