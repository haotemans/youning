<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">关系图谱</span>
        <span class="yt-page-sub mono">{{ data.nodes.length }} 节点 · {{ data.edges.length }} 关系</span>
      </div>
      <n-space align="center" :size="10">
        <n-select :value="sel?.id" :options="data.nodes.map(n => ({ label: n.name || n.id, value: n.id }))"
                  filterable clearable :disabled="loading || !!loadError" placeholder="定位人物节点" size="small" style="width: 200px" @update:value="selectPerson" />
        <n-button size="small" secondary :disabled="loading || !!loadError || !hasNodes" @click="net?.fit({ animation: !reduceMotion })">重置视图</n-button>
      </n-space>
    </div>

    <n-card size="small" class="relations-card">
      <div class="relations-stage">
        <div v-show="hasNodes && !loadError" ref="el" class="graph-canvas" role="img" aria-label="人物关系图谱" :aria-busy="loading" />
        <div v-if="loading" class="graph-status" role="status">正在加载关系图谱…</div>
        <div v-else-if="loadError" class="graph-status" role="alert">
          <span>{{ loadError }}</span>
          <n-button size="small" @click="loadGraph">重新加载</n-button>
        </div>
        <empty-state v-else-if="!hasNodes" title="暂无关系图谱数据" />

        <Transition name="slide-in">
          <aside v-if="sel" :key="sel.id" class="person-drawer">
            <div class="person-drawer-head">
              <img :src="avatarUrl(sel)" class="person-avatar" :alt="sel.name || sel.id" @error="(e) => onImgErr(e, sel)" />
              <div class="person-title-wrap">
                <div class="person-name">{{ sel.name || sel.id }}</div>
                <div class="person-id mono">{{ formatNodeId(sel) }}</div>
              </div>
              <button type="button" class="drawer-close" @click="selectPerson(null)" aria-label="关闭详情">×</button>
            </div>

            <div class="node-meta">
              <div class="meta-row">
                <span class="k">首次记录</span>
                <span class="v mono">{{ fmtTs(sel.first_seen) }}</span>
              </div>
              <div class="meta-row">
                <span class="k">最近活跃</span>
                <span class="v mono">{{ fmtTs(sel.last_seen) }}</span>
              </div>
              <div class="meta-row">
                <span class="k">关联边数</span>
                <span class="v mono">{{ relatedEdges.length }} 条</span>
              </div>
            </div>

            <div class="edge-section-title">关系明细</div>
            <div class="edge-list" v-if="relatedEdges.length">
              <button type="button" v-for="e in relatedEdges" :key="e.from + '→' + e.to" class="edge-item"
                   @click="selectPerson(e.from === sel.id ? e.to : e.from)">
                <div class="edge-peer">
                  <span class="edge-dir mono">{{ e.from === sel.id ? '→' : '←' }}</span>
                  <span class="edge-target">{{ peerName(e.from === sel.id ? e.to : e.from) }}</span>
                </div>
                <div class="edge-metrics mono">
                  <span class="metric-pill">信任 {{ e.trust.toFixed(2) }}</span>
                  <span class="metric-pill">熟悉 {{ e.familiar.toFixed(2) }}</span>
                </div>
              </button>
            </div>
            <div v-else class="edge-empty">暂无与其他节点的关系记录</div>
          </aside>
        </Transition>
      </div>
    </n-card>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { DataSet, Network } from 'vis-network/standalone'
import { api } from '../api'
import { fmtTs } from '../fmt'
import { useUiStore } from '../store/ui'
import EmptyState from '../components/EmptyState.vue'
import mascot from '../assets/mascot.png'

const el = ref(null)
const hasNodes = computed(() => data.value.nodes.length > 0)
const loading = ref(true)
const loadError = ref('')
/** @type {import('vue').Ref<any>} */
const sel = ref(null)
/** @type {import('vue').Ref<{ nodes: any[], edges: any[] }>} */
const data = ref({ nodes: [], edges: [] })
const selfQq = ref(0)
const ui = useUiStore()
const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches
/** @type {any} */
let net = null
/** @type {any} */
let graphNodes = null
/** @type {any} */
let graphEdges = null
const requests = new AbortController()
let disposed = false
let settleTimer
const avatarJobs = new Set()

function destroyGraph() {
  clearTimeout(settleTimer)
  for (const cancel of avatarJobs) cancel()
  avatarJobs.clear()
  net?.destroy()
  net = null
  graphNodes = null
  graphEdges = null
}

// Limit simultaneous avatar requests. Canvas nodes start with local images so
// slow or unreachable QQ servers cannot delay the first useful frame.
async function loadAvatars(nodes, dataset) {
  let index = 0
  async function worker() {
    while (!disposed && graphNodes === dataset && index < nodes.length) {
      const node = nodes[index++]
      const url = avatarUrl(node)
      if (!/^https?:/.test(url)) continue
      await new Promise(resolve => {
        const image = new Image()
        const finish = () => {
          clearTimeout(timeout)
          image.onload = image.onerror = null
          image.src = ''
          avatarJobs.delete(finish)
          resolve(undefined)
        }
        const timeout = setTimeout(finish, 4000)
        avatarJobs.add(finish)
        image.onload = () => {
          if (!disposed && graphNodes === dataset) dataset.update({ id: node.id, image: url })
          finish()
        }
        image.onerror = finish
        image.src = url
      })
    }
  }
  await Promise.all(Array.from({ length: 4 }, worker))
}

const relatedEdges = computed(() =>
  data.value.edges.filter((e) => sel.value && (e.from === sel.value.id || e.to === sel.value.id))
)

function extractQq(id) {
  if (!id) return ''
  const m = String(id).match(/^(?:p_)?(\d{5,12})$/)
  return m ? m[1] : ''
}

function nodeQq(node) {
  const qq = String(node?.qq || '')
  return /^\d{5,12}$/.test(qq) ? qq : extractQq(node?.id)
}

function formatNodeId(node) {
  if (node.id === 'self') return selfQq.value ? `QQ ${selfQq.value}` : '云团'
  const qq = nodeQq(node)
  return qq ? `QQ ${qq}` : String(node.id)
}

function peerName(id) {
  const found = data.value.nodes.find((n) => n.id === id)
  return found ? (found.name || found.id) : id
}

function svgFallbackAvatar(label, isSelf = false) {
  const ch = String(label || '?').replace(/^['"\s]+/, '').charAt(0).toUpperCase() || 'U'
  const bg = isSelf ? '#2563eb' : '#475569'
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120" viewBox="0 0 120 120"><rect width="120" height="120" rx="60" fill="${bg}"/><text x="50%" y="54%" dominant-baseline="middle" text-anchor="middle" fill="#ffffff" font-family="sans-serif" font-size="52" font-weight="600">${ch.replace(/[<>&"']/g, '')}</text></svg>`
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

function avatarUrl(node) {
  if (!node) return ''
  if (/^https?:\/\//.test(node.avatar_url || '')) return node.avatar_url
  if (node.id === 'self') {
    return selfQq.value ? `https://q1.qlogo.cn/g?b=qq&nk=${selfQq.value}&s=100` : mascot
  }
  const qq = nodeQq(node)
  if (qq) return `https://q1.qlogo.cn/g?b=qq&nk=${qq}&s=100`
  return svgFallbackAvatar(node.name || node.id, false)
}

function onImgErr(e, node) {
  const fallback = node?.id === 'self' ? mascot : svgFallbackAvatar(node?.name || node?.id, false)
  if (e.target && e.target.src !== fallback) e.target.src = fallback
}

const selfBorder = () => ui.dark ? '#60a5fa' : '#2563eb'
const peerBorder = () => ui.dark ? '#475569' : '#cbd5e1'
const labelColor = () => ui.dark ? '#f1f5f9' : '#0f172a'

function selectPerson(id) {
  sel.value = id ? (data.value.nodes.find(n => n.id === id) || null) : null
  if (net) {
    net.selectNodes(sel.value ? [sel.value.id] : [])
    if (sel.value) {
      clearTimeout(settleTimer)
      net.setOptions({ physics: false })
      net.focus(sel.value.id, { scale: 1, animation: !reduceMotion })
    }
  }
}

async function loadGraph() {
  destroyGraph()
  sel.value = null
  loading.value = true
  loadError.value = ''
  try {
    const { data: d } = await api.get('/relations', { signal: requests.signal })
    if (disposed) return
    if (!Array.isArray(d?.nodes) || !Array.isArray(d?.edges)) throw new Error('关系图谱数据格式异常')
    data.value = d
    if (!hasNodes.value) return
    await nextTick()
    if (disposed) return

    const degree = Object.create(null)
    for (const e of d.edges) {
      degree[e.from] = (degree[e.from] || 0) + 1
      degree[e.to] = (degree[e.to] || 0) + 1
    }
    const baseSize = (id) => (id === 'self' ? 28 : 24) + Math.min(12, (degree[id] || 0) * 2)
    // Seed a spread-out layout instead of running the blocking
    // improved-layout solver before the canvas has shown anything.
    const ordered = [...d.nodes].sort((a, b) =>
      Number(b.id === 'self') - Number(a.id === 'self') || (degree[b.id] || 0) - (degree[a.id] || 0))
    const positions = new Map(ordered.map((n, i) => {
      const angle = i * Math.PI * (3 - Math.sqrt(5))
      const radius = 95 * Math.sqrt(i)
      return [n.id, { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius }]
    }))

    // 画布内仅展示圆形头像 + 昵称，不挂载悬浮 title 气泡；点击头像再弹出右侧详情卡片
    /** @type {any} */
    const nodes = new DataSet(d.nodes.map((n) => ({
      id: n.id,
      ...positions.get(n.id),
      label: n.name || n.id,
      shape: 'circularImage',
      image: n.id === 'self' ? mascot : svgFallbackAvatar(n.name || n.id),
      brokenImage: n.id === 'self' ? mascot : svgFallbackAvatar(n.name || n.id, false),
      size: baseSize(n.id),
      borderWidth: n.id === 'self' ? 3 : 2,
      borderWidthSelected: 4,
      color: {
        border: n.id === 'self' ? selfBorder() : peerBorder(),
        background: ui.dark ? '#1e293b' : '#f1f5f9',
        highlight: {
          border: selfBorder(),
          background: ui.dark ? '#1e293b' : '#f1f5f9',
        },
        hover: {
          border: selfBorder(),
          background: ui.dark ? '#1e293b' : '#f1f5f9',
        },
      },
      font: {
        color: labelColor(),
        size: 13,
        vadjust: 6,
      },
    })))

    const edges = new DataSet(d.edges.map((e) => ({
      from: e.from,
      to: e.to,
      width: 1.2 + e.familiar * 3.5,
      color: { color: e.from === 'self' || e.to === 'self' ? selfBorder() : '#64748b', opacity: 0.7 },
      arrows: { to: { enabled: true, scaleFactor: 0.6 } },
    })))

    graphNodes = nodes
    graphEdges = edges
    net = new Network(el.value, { nodes, edges }, {
      layout: { improvedLayout: false },
      physics: {
        enabled: !reduceMotion,
        stabilization: false,
        barnesHut: { gravitationalConstant: -5000, springLength: 180, springConstant: 0.04, avoidOverlap: 0.6 },
      },
      edges: { smooth: false },
      interaction: { hover: true, hideEdgesOnDrag: true },
      nodes: {
        font: {
          face: getComputedStyle(document.documentElement).getPropertyValue('--yt-font'),
          size: 13,
          color: labelColor(),
          strokeWidth: 0,
        },
      },
    })

    net.fit({ animation: false })
    // Bound simulation time even for dense/disconnected graphs. Selection and
    // theme updates must never restart it after it has settled.
    const stopLayout = () => {
      clearTimeout(settleTimer)
      net?.setOptions({ physics: false })
      if (!sel.value) net?.fit({ animation: false })
    }
    net.once('stabilized', stopLayout)
    settleTimer = setTimeout(stopLayout, 1500)
    void loadAvatars(d.nodes, nodes)

    net.on('click', (p) => {
      sel.value = p.nodes.length ? d.nodes.find((n) => n.id === p.nodes[0]) : null
    })
  } catch (error) {
    if (!disposed) {
      destroyGraph()
      loadError.value = error.response?.data?.error || error.message || '关系图谱加载失败，请重试'
    }
  } finally {
    if (!disposed) loading.value = false
  }
}

onMounted(() => {
  // Dashboard metadata is optional; it must not hold up the graph request.
  void api.get('/dashboard', { signal: requests.signal }).then(({ data: dash }) => {
    if (!disposed && dash?.self_qq) selfQq.value = dash.self_qq
  }).catch(() => {})
  void loadGraph()
})

onUnmounted(() => {
  disposed = true
  requests.abort()
  destroyGraph()
})

watch(() => ui.dark, () => {
  if (!net || !graphNodes) return
  graphNodes.update(data.value.nodes.map(n => ({
    id: n.id,
    color: {
      border: n.id === 'self' ? selfBorder() : peerBorder(),
      background: ui.dark ? '#1e293b' : '#f1f5f9',
      highlight: { border: selfBorder(), background: ui.dark ? '#1e293b' : '#f1f5f9' },
      hover: { border: selfBorder(), background: ui.dark ? '#1e293b' : '#f1f5f9' },
    },
    font: { color: labelColor(), size: 13, vadjust: 6 },
  })))
  graphEdges.update(graphEdges.get().map(e => ({
    id: e.id,
    color: { color: e.from === 'self' || e.to === 'self' ? selfBorder() : '#64748b', opacity: 0.7 },
  })))
})
</script>

<style scoped>
.relations-card :deep(.n-card__content) {
  padding: 0 !important;
}
.relations-stage {
  position: relative;
  min-height: 420px;
  overflow: hidden;
  border-radius: 12px;
}
.graph-status {
  position: absolute;
  inset: 0;
  min-height: 420px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  gap: 16px;
  color: var(--yt-ink-2);
}
.graph-canvas {
  height: calc(100vh - 165px);
  min-height: 500px;
  border: none;
  border-radius: 12px;
}
.person-drawer {
  position: absolute;
  top: 16px;
  right: 16px;
  bottom: 16px;
  width: 320px;
  max-width: calc(100% - 32px);
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 12px;
  box-shadow: var(--yt-card-shadow-hover);
  padding: 18px;
  display: flex;
  flex-direction: column;
  z-index: 10;
}
.person-drawer-head {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--yt-card-border);
}
.person-avatar {
  width: 44px;
  height: 44px;
  border-radius: 50%;
  object-fit: cover;
  border: 2px solid var(--yt-primary);
  background: var(--yt-soft-bg);
  flex: none;
}
.person-title-wrap {
  flex: 1;
  min-width: 0;
}
.person-name {
  font-size: 15px;
  font-weight: 600;
  color: var(--yt-ink-1);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.person-id {
  font-size: 12px;
  color: var(--yt-ink-3);
  margin-top: 2px;
}
.drawer-close {
  width: 28px;
  height: 28px;
  border-radius: 6px;
  border: 1px solid var(--yt-card-border);
  background: var(--yt-soft-bg);
  color: var(--yt-ink-2);
  font-size: 16px;
  line-height: 1;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
}
.drawer-close:hover {
  color: var(--yt-ink-1);
  border-color: var(--yt-card-border-hover);
}
.node-meta {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 14px 0;
  border-bottom: 1px solid var(--yt-card-border);
}
.meta-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 12.5px;
}
.meta-row .k { color: var(--yt-ink-3); }
.meta-row .v { color: var(--yt-ink-1); }
.edge-section-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--yt-ink-3);
  margin: 14px 0 8px;
}
.edge-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
  overflow-y: auto;
}
.edge-item {
  width: 100%;
  text-align: left;
  font: inherit;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 12px;
  background: var(--yt-elevated);
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.14s ease;
}
.edge-item:hover {
  border-color: var(--yt-primary);
}
.edge-peer {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 600;
  color: var(--yt-ink-1);
}
.edge-dir {
  color: var(--yt-primary);
}
.edge-target {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.edge-metrics {
  display: flex;
  gap: 8px;
  font-size: 11.5px;
  color: var(--yt-ink-2);
}
.metric-pill {
  padding: 1px 6px;
  background: var(--yt-soft-bg);
  border-radius: 4px;
}
.edge-empty {
  font-size: 12.5px;
  color: var(--yt-ink-3);
  text-align: center;
  padding: 24px 0;
}
@media (max-width: 600px) {
  .graph-canvas { min-height: 420px; }
  .person-drawer { top: 8px; right: 8px; bottom: 8px; max-width: calc(100% - 16px); padding: 14px; }
}
@media (prefers-reduced-motion: reduce) {
  .edge-item, .slide-in-enter-active { transition: none; }
}
</style>
