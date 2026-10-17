<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">总览</span>
        <span class="yt-page-sub mono" v-if="d.uptime_secs != null">运行 {{ fmtUptime(d.uptime_secs) }}</span>
      </div>
      <n-space align="center" :size="10">
        <n-tag size="small" :type="d.adapter_connected ? 'success' : 'default'" :bordered="false">
          {{ d.adapter_connected ? 'NapCat 已连接' : 'NapCat 未连接' }}
        </n-tag>
        <n-button size="small" secondary :loading="refreshing" @click="load">刷新</n-button>
      </n-space>
    </div>

    <n-alert v-if="loadError" type="warning" :bordered="false">{{ loadError }}</n-alert>

    <n-grid cols="1 s:2 l:4" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi v-for="s in statCards" :key="s.label">
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" :style="{ background: s.color }" />
            {{ s.label }}
          </div>
          <div class="stat-foot">
            <span class="stat-num mono">
              <CountUp v-if="s.value != null" :value="s.value" />
              <template v-else>—</template>
            </span>
            <Sparkline v-if="s.points.length > 1" :points="s.points" :color="s.color" />
          </div>
        </n-card>
      </n-gi>
    </n-grid>

    <div class="dashboard-columns">
      <div class="dashboard-main">
        <n-card title="24 小时消息分布" size="small">
          <template #header-extra>
            <span class="chart-legend">
              <span><i class="legend-in" />收到</span>
              <span><i class="legend-out" />发送</span>
            </span>
          </template>
          <yt-skeleton v-if="loadingFirst" :height="190" />
          <HourBars v-else-if="hasHourData" :hours="hourSeries" />
          <empty-state v-else title="今日暂无消息记录" />
        </n-card>

        <n-card title="实时事件流" size="small">
          <template #header-extra>
            <router-link to="/trace" class="card-link">全部记录</router-link>
          </template>
          <yt-skeleton v-if="loadingFirst" :height="180" />
          <template v-else>
            <div v-if="recent.length">
              <div v-for="e in recent" :key="e.id ?? e._k" class="mini-event">
                <span class="mini-dot" :style="{ background: kindColor(e.kind) }" />
                <span class="mini-kind">{{ kindLabel(e.kind) }}</span>
                <span class="mini-text" :title="eventSummary(e)">{{ eventSummary(e) }}</span>
                <span class="mini-time">{{ fmtAgo(e.ts) }}</span>
              </div>
            </div>
            <empty-state v-else title="暂无实时事件" />
          </template>
        </n-card>
      </div>

      <n-card title="资源与模型消耗" class="resource-card" size="small">
        <div class="resource-block">
          <div class="resource-label">
            <span>CPU 占用</span>
            <n-tag size="tiny" :type="cpuType" :bordered="false">
              {{ d.cpu_percent == null ? '待采样' : cpuType === 'error' ? '高负载' : cpuType === 'warning' ? '中等' : '正常' }}
            </n-tag>
          </div>
          <div class="resource-value mono">{{ d.cpu_percent == null ? '—' : fmtCpu(d.cpu_percent) }}</div>
          <div class="resource-meter">
            <span :style="{ width: `${Math.min(100, Math.max(0, d.cpu_percent ?? 0))}%`, background: cpuType === 'error' ? 'var(--yt-danger)' : cpuType === 'warning' ? 'var(--yt-warning)' : 'var(--yt-primary)' }" />
          </div>
        </div>

        <div class="resource-block">
          <div class="resource-label">
            <span>进程内存</span>
            <span class="mono resource-cap" v-if="d.mem_total_bytes">/ {{ fmtNum(Math.round(d.mem_total_bytes / 1048576)) }} MB</span>
          </div>
          <div class="resource-value mono">{{ d.mem_rss_bytes == null ? '—' : memMb }} <small>MB</small></div>
        </div>

        <div class="resource-block">
          <div class="resource-label">
            <span>今日 Token 总量</span>
            <span class="mono resource-cap">{{ d.llm_calls_today ?? 0 }} 次调用</span>
          </div>
          <div class="resource-value mono">{{ d.tokens_today == null ? '—' : fmtNum(d.tokens_today) }}</div>
          <div class="token-split">
            <div class="token-item">
              <span class="k">输入</span>
              <span class="v mono">{{ d.tokens_in_today == null ? '—' : fmtNum(d.tokens_in_today) }}</span>
            </div>
            <div class="token-item">
              <span class="k">输出</span>
              <span class="v mono">{{ d.tokens_out_today == null ? '—' : fmtNum(d.tokens_out_today) }}</span>
            </div>
          </div>
        </div>
        <div class="resource-block last">
          <div class="resource-label">
            <span>累计 Token 总量</span>
            <span class="mono resource-cap">{{ d.llm_calls_total ?? 0 }} 次调用</span>
          </div>
          <div class="resource-value mono">{{ d.tokens_total == null ? '—' : fmtNum(d.tokens_total) }}</div>
          <div class="token-split">
            <div class="token-item">
              <span class="k">输入</span>
              <span class="v mono">{{ d.tokens_in_total == null ? '—' : fmtNum(d.tokens_in_total) }}</span>
            </div>
            <div class="token-item">
              <span class="k">输出</span>
              <span class="v mono">{{ d.tokens_out_total == null ? '—' : fmtNum(d.tokens_out_total) }}</span>
            </div>
          </div>
        </div>
      </n-card>
    </div>
  </div>
</template>

<script>
const TREND_CAP = 40
const trend = { inToday: [], outToday: [], decision: [], tasks: [], tokens: [], mem: [] }
</script>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'
import { useUiStore } from '../store/ui'
import { kindColor, kindLabel, eventSummary, fmtAgo, fmtUptime } from '../fmt'
import Sparkline from '../components/Sparkline.vue'
import HourBars from '../components/HourBars.vue'
import EmptyState from '../components/EmptyState.vue'
import CountUp from '../components/CountUp.vue'
import YtSkeleton from '../components/YtSkeleton.vue'
import { subscribeWs } from '../ws'

/** @type {import('vue').Ref<Record<string, any>>} */
const d = ref({})
/** @type {import('vue').Ref<any[]>} */
const recent = ref([])
/** @type {import('vue').Ref<{ in: number, out: number }[]>} */
const hourSeries = ref(Array.from({ length: 24 }, () => ({ in: 0, out: 0 })))
const hasHourData = ref(false)
const loadingFirst = ref(true)
const refreshing = ref(false)
const loadError = ref('')
const ui = useUiStore()
let timer = null

function sample(key, v) {
  if (typeof v !== 'number') return
  const arr = trend[key]
  arr.push(v)
  if (arr.length > TREND_CAP) arr.shift()
}

const statCards = computed(() => [
  { label: '今日接收消息', value: d.value.messages_in_today, points: trend.inToday, color: 'var(--yt-primary)' },
  { label: '今日发送回复', value: d.value.messages_out_today, points: trend.outToday, color: 'var(--yt-ok)' },
  { label: '策略决策次数', value: d.value.decision_calls_today, points: trend.decision, color: 'var(--yt-violet)' },
  { label: '运行中任务', value: d.value.active_tasks, points: trend.tasks, color: 'var(--yt-warning)' },
])

const memMb = computed(() => Math.round((d.value.mem_rss_bytes ?? 0) / 1048576))
const cpuType = computed(() => {
  const c = d.value.cpu_percent ?? 0
  if (c >= 80) return 'error'
  if (c >= 50) return 'warning'
  return 'success'
})

function fmtNum(n) {
  if (n == null) return '0'
  if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M'
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'k'
  return String(n)
}
function fmtCpu(c) {
  return (c ?? 0).toFixed(1) + '%'
}

function aggregateHours(events) {
  const midnight = new Date()
  midnight.setHours(0, 0, 0, 0)
  const t0 = Math.floor(midnight.getTime() / 1000)
  const buckets = Array.from({ length: 24 }, () => ({ in: 0, out: 0 }))
  let any = false
  for (const e of events) {
    if (!e.ts || e.ts < t0) continue
    const h = new Date(e.ts * 1000).getHours()
    if (e.kind === 'MessageReceived') { buckets[h].in++; any = true }
    else if (e.kind === 'BubbleSent') { buckets[h].out++; any = true }
  }
  return { buckets, any }
}

async function load() {
  if (refreshing.value) return
  refreshing.value = true
  loadError.value = ''
  try {
    const { data } = await api.get('/dashboard')
    d.value = data
    ui.setMood(data.mood)
    sample('inToday', data.messages_in_today)
    sample('outToday', data.messages_out_today)
    sample('decision', data.decision_calls_today)
    sample('tasks', data.active_tasks)
    sample('tokens', data.tokens_today)
    sample('mem', Math.round((data.mem_rss_bytes ?? 0) / 1048576))
  } catch {
    loadError.value = '运行数据获取失败，请检查后端服务状态。'
  }
  try {
    const { data } = await api.get('/events?limit=500')
    recent.value = data.events.slice(0, 8)
    const { buckets, any } = aggregateHours(data.events)
    hourSeries.value = buckets
    hasHourData.value = any
  } catch {
    loadError.value = '事件流获取失败，请检查后端服务状态。'
  }
  loadingFirst.value = false
  refreshing.value = false
}

function onWsEvent(ev) {
  if (!ev || !ev.kind) return
  const now = Math.floor(Date.now() / 1000)
  if (ev.kind === 'MessageReceived') {
    d.value.messages_in_today = (d.value.messages_in_today ?? 0) + 1
    sample('inToday', d.value.messages_in_today)
  } else if (ev.kind === 'BubbleSent') {
    d.value.messages_out_today = (d.value.messages_out_today ?? 0) + 1
    sample('outToday', d.value.messages_out_today)
  } else if (ev.kind === 'DecisionMade') {
    d.value.decision_calls_today = (d.value.decision_calls_today ?? 0) + 1
    sample('decision', d.value.decision_calls_today)
    if (ev.payload?.mood) ui.setMood(ev.payload.mood)
  } else if (ev.kind === 'TaskCreated') {
    d.value.active_tasks = (d.value.active_tasks ?? 0) + 1
    sample('tasks', d.value.active_tasks)
  } else if (ev.kind === 'TaskFinished') {
    d.value.active_tasks = Math.max(0, (d.value.active_tasks ?? 0) - 1)
    sample('tasks', d.value.active_tasks)
  }
  const item = { _k: Math.random(), kind: ev.kind, payload: ev.payload ?? ev, ts: ev.ts ?? now }
  recent.value = [item, ...recent.value].slice(0, 8)
  const midnight = new Date(); midnight.setHours(0, 0, 0, 0)
  const t0 = Math.floor(midnight.getTime() / 1000)
  if (item.ts >= t0) {
    const h = new Date(item.ts * 1000).getHours()
    if (ev.kind === 'MessageReceived') { hourSeries.value[h].in++; hasHourData.value = true }
    else if (ev.kind === 'BubbleSent') { hourSeries.value[h].out++; hasHourData.value = true }
  }
}

let off = null
onMounted(() => {
  load()
  timer = setInterval(load, 60000)
  off = subscribeWs(onWsEvent)
})
onUnmounted(() => { clearInterval(timer); off && off() })
</script>

<style scoped>
.dashboard-columns {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 300px;
  gap: 16px;
  align-items: start;
}
.dashboard-main {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 16px;
  min-width: 0;
}
.dashboard-main > .n-card,
.resource-card {
  min-width: 0;
}
.card-link {
  color: var(--yt-primary);
  text-decoration: none;
  font-size: 12.5px;
  font-weight: 500;
}
.card-link:hover {
  text-decoration: underline;
}
.chart-legend,
.chart-legend > span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--yt-ink-2);
}
.chart-legend {
  gap: 14px;
}
.chart-legend i {
  width: 10px;
  height: 10px;
  border-radius: 2px;
}
.legend-in { background: var(--yt-chart-in); }
.legend-out { background: var(--yt-chart-out); }

.resource-block {
  padding-bottom: 16px;
  margin-bottom: 16px;
  border-bottom: 1px solid var(--yt-card-border);
}
.resource-block.last {
  padding-bottom: 0;
  margin-bottom: 0;
  border-bottom: none;
}
.resource-label {
  display: flex;
  justify-content: space-between;
  align-items: center;
  color: var(--yt-ink-3);
  font-size: 12.5px;
  font-weight: 500;
}
.resource-cap {
  font-size: 12px;
  color: var(--yt-ink-3);
}
.resource-value {
  margin: 8px 0 10px;
  font-size: 24px;
  font-weight: 600;
  color: var(--yt-ink-1);
}
.resource-value small {
  font-size: 12.5px;
  font-weight: 500;
  color: var(--yt-ink-3);
}
.resource-meter {
  height: 6px;
  border-radius: 3px;
  overflow: hidden;
  background: var(--yt-soft-bg);
}
.resource-meter span {
  display: block;
  height: 100%;
  transition: width 0.3s ease;
}
.token-split {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
  margin-top: 8px;
}
.token-item {
  background: var(--yt-soft-bg);
  border-radius: 8px;
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.token-item .k {
  font-size: 11.5px;
  color: var(--yt-ink-3);
}
.token-item .v {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--yt-ink-1);
}
@media (max-width: 1080px) {
  .dashboard-columns { grid-template-columns: minmax(0, 1fr); }
}
</style>
