<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">决策追踪</span>
        <span class="yt-page-sub mono">{{ items.length }} 条记录</span>
      </div>
      <n-button size="small" secondary :loading="loading" @click="reset">刷新</n-button>
    </div>

    <div class="yt-toolbar">
      <n-select v-model:value="kind" :options="kindOptions" clearable placeholder="事件类型" size="small" style="width: 190px" @update:value="reset" />
      <n-input v-model:value="chat" placeholder="会话 ID" size="small" style="width: 180px" @keyup.enter="reset" />
      <n-button size="small" type="primary" secondary @click="reset">筛选</n-button>
      <span class="spacer" />
      <n-tag size="small" :type="wsConnected ? 'success' : 'default'" :bordered="false">
        {{ wsConnected ? '实时流已连接' : '实时流已断开' }}
      </n-tag>
    </div>

    <div class="trace-workspace">
      <section class="trace-events">
        <div v-if="items.length" class="tl">
          <div v-for="e in items" :key="e.id ?? e._k" class="tl-item" :class="{ active: sel === e }" @click="sel = e">
            <div class="tl-rail"><span class="tl-dot" :style="{ background: kindColor(e.kind) }" /></div>
            <div class="tl-card" :class="{ 'tl-decision': e.kind === 'DecisionMade' }"
                 :style="e.kind === 'DecisionMade' ? { '--action-c': actionColor(e.payload?.action) } : {}">
              <div class="tl-head">
                <span class="tl-badge" :style="badgeStyle(e.kind)">{{ e.kind }}</span>
                <span v-if="e.kind === 'DecisionMade' && e.payload?.action" class="tl-badge"
                      :style="badgeStyleFor(e.payload?.action, actionColor(e.payload?.action))">
                  {{ e.payload?.action }}
                </span>
                <span class="tl-time">{{ fmtAgo(e.ts) }}</span>
              </div>
              <div class="tl-text">{{ eventSummary(e) }}</div>
              <div v-if="e.payload?.chat_id" class="tl-chat">Chat {{ e.payload.chat_id }}</div>
            </div>
          </div>
        </div>
        <n-card v-else size="small">
          <empty-state title="暂无匹配事件" />
        </n-card>
        <n-button v-if="items.length" block size="small" secondary style="margin-top: 10px" :loading="loading" @click="loadMore">
          加载更早记录
        </n-button>
      </section>

      <aside class="trace-detail">
        <template v-if="sel">
          <div class="detail-header">
            <n-tag size="small" :bordered="false" :color="badgeStyleObj(sel.kind)">{{ sel.kind }}</n-tag>
            <span v-if="sel.ts" class="tl-chat" style="margin-top: 0">{{ fmtTs(sel.ts) }}</span>
          </div>
          <template v-if="sel.kind === 'DecisionMade'">
            <n-space size="small" style="margin-bottom: 10px">
              <n-tag size="small" :bordered="false" :color="actionTagColor(sel.payload.action)">{{ sel.payload.action }}</n-tag>
              <n-tag v-if="sel.payload.mood" size="small" :bordered="false">{{ sel.payload.mood }}</n-tag>
              <n-tag v-if="sel.payload.fallback" size="small" :bordered="false" type="error">Fallback</n-tag>
            </n-space>
            <n-alert v-if="sel.payload.reason" type="info" :bordered="false" style="margin-bottom: 10px">
              {{ sel.payload.reason }}
            </n-alert>
            <template v-if="sel.payload.policy">
              <n-space size="small" style="margin-bottom: 10px">
                <n-tag size="small" :bordered="false">建议动作: {{ sel.payload.policy.suggested_action }}</n-tag>
                <n-tag v-if="sel.payload.reply_mode" size="small" :bordered="false">模式: {{ sel.payload.reply_mode }}</n-tag>
                <n-tag v-if="sel.payload.policy.evidence_valid === false" size="small" :bordered="false" type="warning">依据未通过</n-tag>
              </n-space>
              <n-alert v-if="sel.payload.policy.notes?.length" type="warning" :bordered="false" style="margin-bottom: 10px">
                {{ sel.payload.policy.notes.join('；') }}
              </n-alert>
            </template>
          </template>
          <json-view :data="sel.payload" />
        </template>
        <empty-state v-else title="选择左侧事件查看详情" />
      </aside>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'
import { subscribeWs, wsConnected } from '../ws'
import { kindColor, actionColor, eventSummary, fmtAgo, fmtTs } from '../fmt'
import JsonView from '../components/JsonView.vue'
import EmptyState from '../components/EmptyState.vue'

/** @type {import('vue').Ref<any[]>} */
const items = ref([])
/** @type {import('vue').Ref<any>} */
const sel = ref(null)
const kind = ref(null)
const chat = ref('')
const loading = ref(false)

const kindOptions = [
  'MessageReceived', 'DecisionMade', 'BubbleSent', 'ReplyInterrupted',
  'ConsolidationDone', 'MemoryWritten', 'MoodChanged', 'ConfigReloaded',
].map((k) => ({ label: k, value: k }))

function badgeStyle(k) {
  return badgeStyleFor(k, kindColor(k))
}
function badgeStyleFor(_k, c) {
  return { color: c, background: `color-mix(in srgb, ${c} 10%, transparent)`, border: `1px solid color-mix(in srgb, ${c} 24%, transparent)` }
}
function badgeStyleObj(k) {
  const c = kindColor(k)
  return { color: `color-mix(in srgb, ${c} 10%, transparent)`, textColor: c }
}
function actionTagColor(a) {
  const c = actionColor(a)
  return { color: `color-mix(in srgb, ${c} 10%, transparent)`, textColor: c }
}

async function fetchPage(beforeId) {
  loading.value = true
  try {
    const params = new URLSearchParams({ limit: '50' })
    if (kind.value) params.set('kind', kind.value)
    if (chat.value) params.set('chat_id', chat.value)
    if (beforeId) params.set('before_id', String(beforeId))
    const { data } = await api.get(`/events?${params}`)
    return data.events
  } finally {
    loading.value = false
  }
}

async function reset() {
  items.value = await fetchPage()
  if (items.value.length && !sel.value) sel.value = items.value[0]
}
async function loadMore() {
  const oldest = items.value[items.value.length - 1]
  if (!oldest?.id) return
  items.value = items.value.concat(await fetchPage(oldest.id))
}

let off = null
onMounted(async () => {
  await reset()
  off = subscribeWs((ev) => {
    if (kind.value && ev.kind !== kind.value) return
    if (chat.value && ev.chat_id !== chat.value && ev.payload?.chat_id !== chat.value) return
    items.value.unshift({ _k: Math.random(), kind: ev.kind, payload: ev, ts: Math.floor(Date.now() / 1000) })
    if (items.value.length > 300) items.value.pop()
  })
})
onUnmounted(() => off && off())
</script>

<style scoped>
.trace-workspace {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 440px;
  gap: 16px;
  align-items: start;
}
.trace-events { min-width: 0; }
.trace-detail {
  min-width: 0;
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 12px;
  padding: 18px;
  box-shadow: var(--yt-card-shadow);
}
.detail-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}
@media (max-width: 1080px) {
  .trace-workspace { grid-template-columns: 1fr; }
}
</style>
