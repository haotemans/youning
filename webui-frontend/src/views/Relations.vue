<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">关系图谱</span>
        <span class="yt-page-sub mono">{{ data.nodes.length }} 节点 · {{ data.edges.length }} 关系</span>
      </div>
      <n-space align="center" :size="10">
        <n-select :value="sel?.id" :options="data.nodes.map(n => ({ label: n.name || n.id, value: n.id }))"
                  clearable placeholder="定位人物节点" size="small" style="width: 200px" @update:value="selectPerson" />
        <n-button size="small" secondary :disabled="!hasNodes" @click="net?.fit({ animation: false })">重置视图</n-button>
      </n-space>
    </div>

    <div class="relations-layout">
      <div class="relations-main">
        <n-card size="small" title="拓扑视图">
          <template #header-extra>
            <div class="graph-legend">
              <span><i class="self-node" />云团</span>
              <span><i class="peer-node" />群成员</span>
            </div>
          </template>
          <div v-show="hasNodes" ref="el" class="graph-canvas" />
          <empty-state v-if="!hasNodes" title="暂无关系图谱数据" />
        </n-card>
      </div>

      <div>
        <Transition name="slide-in" mode="out-in">
          <n-card v-if="sel" :key="sel.id" size="small" :title="sel.name || sel.id">
            <template #header-extra>
              <span class="mono" style="font-size: 12px; color: var(--yt-ink-3)">{{ sel.id }}</span>
            </template>
            <div class="node-meta">
              <div class="meta-row"><span class="k">首次记录</span><span class="v mono">{{ fmtTs(sel.first_seen) }}</span></div>
              <div class="meta-row"><span class="k">最近活跃</span><span class="v mono">{{ fmtTs(sel.last_seen) }}</span></div>
            </div>
            <n-divider style="margin: 12px 0" />
            <div class="edge-list" v-if="relatedEdges.length">
              <div v-for="e in relatedEdges" :key="e.from + '→' + e.to" class="edge-item">
                <span class="mono edge-target">{{ e.from === sel.id ? `→ ${e.to}` : `← ${e.from}` }}</span>
                <span class="mono edge-metrics">信任 {{ e.trust.toFixed(2) }} · 熟悉 {{ e.familiar.toFixed(2) }}</span>
              </div>
            </div>
            <empty-state v-else title="暂无关联边" />
          </n-card>
          <n-card v-else size="small" title="节点详情">
            <empty-state title="点击画布节点查看关系" />
          </n-card>
        </Transition>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { DataSet, Network } from 'vis-network/standalone'
import { api } from '../api'
import { fmtTs } from '../fmt'
import { useUiStore } from '../store/ui'
import EmptyState from '../components/EmptyState.vue'

const el = ref(null)
const hasNodes = ref(true)
/** @type {import('vue').Ref<any>} */
const sel = ref(null)
/** @type {import('vue').Ref<{ nodes: any[], edges: any[] }>} */
const data = ref({ nodes: [], edges: [] })
const ui = useUiStore()
/** @type {any} */
let net = null

const relatedEdges = computed(() =>
  data.value.edges.filter((e) => sel.value && (e.from === sel.value.id || e.to === sel.value.id))
)

const selfColor = () => ui.dark ? '#60a5fa' : '#2563eb'
const peerColor = () => ui.dark ? '#475569' : '#94a3b8'
/** @type {any} */
let graphNodes = null
/** @type {any} */
let graphEdges = null

function selectPerson(id) {
  sel.value = data.value.nodes.find(n => n.id === id) || null
  if (net) net.selectNodes(id == null ? [] : [id])
}

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c])

function labelColor() {
  return ui.dark ? '#e2e8f0' : '#1e293b'
}

onMounted(async () => {
  const { data: d } = await api.get('/relations')
  data.value = d
  hasNodes.value = d.nodes.length > 0
  if (!hasNodes.value) return

  const degree = {}
  for (const e of d.edges) {
    degree[e.from] = (degree[e.from] || 0) + 1
    degree[e.to] = (degree[e.to] || 0) + 1
  }
  const baseSize = (id) => 12 + Math.min(18, (degree[id] || 0) * 3)
  const tip = (n) =>
    `<b>${esc(n.name || n.id)}</b><br/>ID: ${esc(n.id)}<br/>最近: ${fmtTs(n.last_seen)}`

  /** @type {any} */
  const nodes = new DataSet(d.nodes.map((n) => ({
    id: n.id,
    label: n.name || n.id,
    title: tip(n),
    color: n.id === 'self' ? selfColor() : peerColor(),
    font: { color: n.id === 'self' ? (ui.dark ? '#0f172a' : '#ffffff') : labelColor() },
    shape: n.id === 'self' ? 'box' : 'dot',
    size: baseSize(n.id),
    _base: baseSize(n.id),
  })))
  const edges = new DataSet(d.edges.map((e) => ({
    from: e.from,
    to: e.to,
    width: 1 + e.familiar * 4,
    color: { color: e.from === 'self' || e.to === 'self' ? selfColor() : '#94a3b8', opacity: 0.75 },
    title: `信任 ${e.trust.toFixed(2)} / 熟悉 ${e.familiar.toFixed(2)}`,
    arrows: 'to',
  })))

  graphNodes = nodes
  graphEdges = edges
  net = new Network(el.value, { nodes, edges }, {
    physics: { stabilization: true },
    edges: { smooth: true },
    interaction: { hover: true, tooltipDelay: 120 },
    nodes: {
      borderWidth: 2,
      font: { face: getComputedStyle(document.documentElement).getPropertyValue('--yt-font'), size: 13, color: labelColor(), strokeWidth: 0 },
    },
  })

  net.on('click', (p) => {
    sel.value = p.nodes.length ? d.nodes.find((n) => n.id === p.nodes[0]) : null
  })
})

onUnmounted(() => { net?.destroy(); net = null })

watch(() => ui.dark, () => {
  if (!net) return
  net.setOptions({ nodes: { font: { color: labelColor() } } })
  graphNodes.update(data.value.nodes.map(n => ({
    id: n.id,
    color: n.id === 'self' ? selfColor() : peerColor(),
    font: { color: n.id === 'self' ? (ui.dark ? '#0f172a' : '#ffffff') : labelColor() },
  })))
  graphEdges.update(graphEdges.get().map(e => ({
    id: e.id,
    color: { color: e.from === 'self' || e.to === 'self' ? selfColor() : '#94a3b8', opacity: 0.75 },
  })))
})
</script>

<style scoped>
.relations-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 340px;
  gap: 16px;
  align-items: start;
}
.relations-main { min-width: 0; }
.graph-legend {
  display: flex;
  gap: 14px;
  align-items: center;
  font-size: 12px;
  color: var(--yt-ink-2);
}
.graph-legend span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.graph-legend i {
  width: 10px;
  height: 10px;
  display: inline-block;
}
.self-node { background: var(--yt-primary); border-radius: 2px; }
.peer-node { background: var(--yt-chart-in); border-radius: 50%; }
.node-meta {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.meta-row {
  display: flex;
  justify-content: space-between;
  font-size: 12.5px;
}
.meta-row .k { color: var(--yt-ink-3); }
.meta-row .v { color: var(--yt-ink-1); }
.edge-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  max-height: 360px;
  overflow-y: auto;
}
.edge-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 10px;
  background: var(--yt-soft-bg);
  border-radius: 6px;
  font-size: 12px;
}
.edge-target { font-weight: 600; color: var(--yt-ink-1); }
.edge-metrics { color: var(--yt-ink-2); }
@media (max-width: 1080px) {
  .relations-layout { grid-template-columns: 1fr; }
}
</style>
