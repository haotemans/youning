<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">插件管理</span>
        <span class="yt-page-sub mono">{{ plugins.length }} 个插件</span>
      </div>
      <span class="yt-page-sub">点开插件查看功能与配置</span>
    </div>

    <n-alert v-if="dirty.size" type="warning" :bordered="false">
      {{ dirty.size }} 个插件有待生效的变更，重启后端服务后生效。
    </n-alert>
    <n-alert v-if="error && !showDetail" type="error" :bordered="false">{{ error }}</n-alert>

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

    <n-modal v-model:show="showDetail" preset="card" :title="detail?.display_name || detail?.name" style="max-width: 600px">
      <template #header-extra>
        <n-tag size="tiny" :bordered="false" type="info" v-if="detail?.version">v{{ detail.version }}</n-tag>
      </template>
      <div v-if="detail" class="detail">
        <n-alert v-if="error" type="error" :bordered="false">{{ error }}</n-alert>
        <div class="detail-row">
          <span class="k">运行状态</span>
          <span class="v">
            <n-tag size="tiny" :bordered="false" type="success" v-if="detail.enabled && detail.loaded">已加载</n-tag>
            <n-tag size="tiny" :bordered="false" type="warning" v-else-if="detail.enabled && !detail.loaded">待重启</n-tag>
            <n-tag size="tiny" :bordered="false" v-else>已禁用</n-tag>
          </span>
        </div>
        <n-alert v-if="detail.error" type="error" :bordered="false">{{ detail.error }}</n-alert>
        <n-alert v-if="detail.pending_restart" type="info" :bordered="false">配置已保存，重启服务后生效。</n-alert>
        <div class="detail-block" v-if="detail.config_fields?.length">
          <n-form label-placement="top" :show-feedback="false">
            <n-form-item v-for="field in detail.config_fields" :key="field.key" :label="field.label">
              <n-select v-if="field.kind === 'choice'" v-model:value="configDraft[field.key]" :options="field.options || []" />
              <n-switch v-else-if="field.kind === 'boolean'" v-model:value="configDraft[field.key]" />
              <n-input-number v-else-if="field.kind === 'integer'" v-model:value="configDraft[field.key]" :precision="0" style="width: 100%" />
              <div v-else class="field-input">
                <n-input v-model:value="configDraft[field.key]" :type="field.secret ? 'password' : 'text'"
                  :disabled="saving || !!clearSecrets[field.key]" show-password-on="click"
                  :placeholder="field.secret ? '留空保留已保存的值' : ''" autocomplete="off" />
                <n-checkbox v-if="field.secret" v-model:checked="clearSecrets[field.key]" :disabled="saving">清除已保存的值</n-checkbox>
              </div>
            </n-form-item>
          </n-form>
          <n-space align="center" style="margin-top: 14px">
            <n-button type="primary" size="small" :loading="saving" @click="saveConfig">保存配置</n-button>
            <span v-if="saved" class="save-note">{{ saved }}</span>
          </n-space>
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
const configDraft = ref({})
const clearSecrets = ref({})
const saving = ref(false)
const saved = ref('')
const error = ref('')

async function load() {
  try {
    const { data } = await api.get('/plugins/list')
    plugins.value = data.plugins || []
    dirty.value = new Set(plugins.value.filter(p => p.pending_restart).map(p => p.name))
  } catch (e) {
    error.value = e.response?.data?.error || '插件列表加载失败'
  }
}

async function toggle(p, v) {
  try {
    error.value = ''
    await api.post('/plugins/toggle', { name: p.name, enabled: v })
    await load()
  } catch (e) {
    error.value = e.response?.data?.error || '插件状态保存失败'
  }
}

function openDetail(p) {
  detail.value = p
  configDraft.value = Object.fromEntries((p.config_fields || []).map(f => [f.key, f.secret ? '' : (p.settings?.config?.[f.key] ?? f.default)]))
  clearSecrets.value = {}
  saved.value = ''
  error.value = ''
  showDetail.value = true
}

async function saveConfig() {
  if (!detail.value || saving.value) return
  saving.value = true
  error.value = ''
  saved.value = ''
  try {
    const settings = JSON.parse(JSON.stringify(detail.value.settings))
    for (const field of detail.value.config_fields || []) {
      const value = configDraft.value[field.key]
      if (field.secret) {
        delete settings.config[field.key]
        if (clearSecrets.value[field.key]) settings.config[field.key] = ''
        else if (value) settings.config[field.key] = value
      } else settings.config[field.key] = value
    }
    await api.post('/plugins/config', { name: detail.value.name, settings })
    const name = detail.value.name
    await load()
    detail.value = plugins.value.find(p => p.name === name) || detail.value
    for (const field of detail.value.config_fields || []) {
      if (field.secret) configDraft.value[field.key] = ''
    }
    clearSecrets.value = {}
    saved.value = '已保存，重启服务后生效'
  } catch (e) {
    error.value = e.response?.data?.error || '插件配置保存失败'
  } finally {
    saving.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.detail { display: flex; flex-direction: column; gap: 12px; }
.field-input { display: flex; flex-direction: column; gap: 8px; width: 100%; }
.save-note { font-size: 12px; color: var(--yt-ink-3); }
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
