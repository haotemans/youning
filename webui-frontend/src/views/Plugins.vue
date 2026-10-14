<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">插件管理</span>
        <span class="yt-page-sub mono">{{ plugins.length }} 个插件</span>
      </div>
      <n-button size="small" secondary @click="load">刷新</n-button>
    </div>

    <n-alert v-if="dirty.size" type="warning" :bordered="false">
      已变更 {{ dirty.size }} 个插件状态（{{ [...dirty].join('、') }}），重启后端服务后生效。
    </n-alert>

    <div v-if="plugins.length" class="cap-grid">
      <PluginCard
        v-for="p in plugins"
        :key="p.name"
        :plugin="p"
        :dirty="dirty.has(p.name)"
        @toggle="toggle"
        @open="openDetail"
      />
    </div>
    <n-card v-else size="small">
      <empty-state title="暂无已安装插件" />
    </n-card>

    <n-modal v-model:show="showDetail" preset="card" :title="detail?.name" style="max-width: 600px">
      <template #header-extra>
        <n-tag size="tiny" :bordered="false" type="info" v-if="detail?.version">v{{ detail.version }}</n-tag>
      </template>
      <div v-if="detail" class="detail">
        <div class="detail-row">
          <span class="k">运行状态</span>
          <span class="v">
            <n-tag size="tiny" :bordered="false" type="success" v-if="detail.enabled && detail.loaded">已加载</n-tag>
            <n-tag size="tiny" :bordered="false" type="warning" v-else-if="detail.enabled && !detail.loaded">待重启</n-tag>
            <n-tag size="tiny" :bordered="false" v-else>已禁用</n-tag>
          </span>
        </div>
        <div class="detail-row">
          <span class="k">功能描述</span>
          <span class="v">{{ detail.description || '—' }}</span>
        </div>
        <div class="detail-row" v-if="detail.data_dir">
          <span class="k">数据目录</span>
          <span class="v mono">{{ detail.data_dir }}</span>
        </div>
        <div class="detail-block" v-if="(detail.skills || []).length">
          <div class="cap-block-label">Skills ({{ detail.skills.length }})</div>
          <div class="cap-block-list">
            <div v-for="s in detail.skills" :key="s.name" class="cap-block-item">
              <span class="chip skill mono">{{ s.name }}</span>
              <span class="cap-block-desc">{{ s.description }}</span>
            </div>
          </div>
        </div>
        <div class="detail-block" v-if="(detail.tools || []).length">
          <div class="cap-block-label">Tools ({{ detail.tools.length }})</div>
          <div class="cap-block-list">
            <div v-for="t in detail.tools" :key="t.name" class="cap-block-item">
              <span class="chip tool mono">{{ t.name }}</span>
            </div>
          </div>
        </div>
      </div>
    </n-modal>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'
import PluginCard from '../components/PluginCard.vue'

const plugins = ref([])
const dirty = ref(new Set())
const detail = ref(null)
const showDetail = ref(false)

async function load() {
  const { data } = await api.get('/plugins/list')
  plugins.value = data.plugins || []
  dirty.value = new Set(plugins.value.filter(p => p.enabled !== p.loaded).map(p => p.name))
}

async function toggle(p, v) {
  try {
    await api.post('/plugins/toggle', { name: p.name, enabled: v })
    p.enabled = v
    if (v !== p.loaded) dirty.value.add(p.name)
    else dirty.value.delete(p.name)
    dirty.value = new Set(dirty.value)
  } catch (e) {
    console.error('toggle 失败', e)
  }
}

function openDetail(p) {
  detail.value = p
  showDetail.value = true
}

onMounted(load)
</script>

<style scoped>
.detail { display: flex; flex-direction: column; gap: 12px; }
.detail-row { display: flex; gap: 14px; align-items: baseline; }
.detail-row .k {
  flex: none; width: 72px;
  font-size: 12px; color: var(--yt-ink-3);
}
.detail-row .v { font-size: 13px; color: var(--yt-ink-1); }
.detail-block {
  border-top: 1px solid var(--yt-card-border);
  padding-top: 12px;
}
.cap-block-label {
  font-size: 12px;
  color: var(--yt-ink-3);
  margin-bottom: 8px;
  font-weight: 600;
}
.cap-block-list { display: flex; flex-direction: column; gap: 8px; }
.cap-block-item { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.cap-block-desc { font-size: 12.5px; color: var(--yt-ink-2); }
</style>
