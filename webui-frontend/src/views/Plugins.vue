<template>
  <div class="yt-page plugins-page">
    <header class="plugins-header">
      <div><h1>插件</h1><p>各有所长，为云团添一份本领。</p></div>
      <div class="plugins-summary" aria-live="polite"><span class="summary-dot" />{{ readyCount }} 个已启用<span class="summary-divider" />{{ plugins.length }} 个已安装</div>
    </header>

    <div class="plugins-toolbar">
      <div class="plugin-filters" role="group" aria-label="筛选插件">
        <button v-for="item in filters" :key="item.value" :class="{ selected: filter === item.value }" :aria-pressed="filter === item.value" @click="filter = item.value">{{ item.label }}<span>{{ item.count }}</span></button>
      </div>
      <n-input v-model:value="query" class="plugin-search" clearable placeholder="搜索名称或功能" aria-label="搜索插件">
        <template #prefix><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 4.5 4.5" /></svg></template>
      </n-input>
    </div>

    <n-alert v-if="loadError" type="error" :bordered="false" class="plugin-alert">{{ loadError }} <n-button text size="small" @click="load(true)">重试</n-button></n-alert>
    <n-alert v-if="actionError && !showDetail" type="error" :bordered="false" closable @close="actionError = ''">{{ actionError }}</n-alert>
    <div v-if="initialLoading" class="plugins-grid" aria-label="正在加载插件"><n-skeleton v-for="i in 3" :key="i" height="310px" style="border-radius:16px" /></div>
    <div v-else-if="filtered.length" class="plugins-grid">
      <PluginCard v-for="plugin in filtered" :key="plugin.name" :plugin="plugin" :busy="!!pending[plugin.name]" @toggle="toggle" @open="openDetail" />
    </div>
    <div v-else-if="!loadError" class="plugins-empty"><EmptyState :title="plugins.length ? '没有符合条件的插件' : '暂无已安装插件'" /><p>{{ plugins.length ? '换一个名称、功能关键词或筛选条件试试。' : '安装完成后，插件会显示在这里。' }}</p></div>
    <p class="plugins-footnote" v-if="plugins.length">状态自动更新。配置会在当前任务结束后应用。</p>

    <n-drawer v-model:show="showDetail" :width="drawerWidth" :auto-focus="true" class="plugin-drawer">
      <n-drawer-content v-if="detail" closable :native-scrollbar="false" :body-content-style="{ padding: '0 26px 24px' }">
        <template #header><div class="drawer-heading"><span class="drawer-name">{{ detail.display_name || detail.name }}</span><span class="drawer-purpose">{{ detail.presentation?.purpose || '插件详情' }}</span></div></template>
        <div class="drawer-status"><span class="detail-state" :class="status.tone"><i />{{ status.label }}</span><span>版本 {{ detail.version }}</span></div>
        <n-alert v-if="actionError" type="error" :bordered="false" class="plugin-alert">{{ actionError }}</n-alert>
        <n-alert v-if="detail.error" type="warning" :bordered="false" class="plugin-alert">{{ detail.error }}</n-alert>
        <n-alert v-if="busy" type="info" :bordered="false" class="plugin-alert">正在应用设置。当前任务会继续完成，页面会自动显示结果。</n-alert>
        <n-alert v-else-if="detail.pending_restart" type="error" :bordered="false" class="plugin-alert">插件未能安全停止，需要重启云团服务后再加载。</n-alert>
        <n-alert v-else-if="detail.pending_reload" type="warning" :bordered="false" class="plugin-alert">新配置尚未生效，请修正配置后重新保存。</n-alert>
        <n-tabs v-model:value="activeTab" type="line" animated>
          <n-tab-pane name="overview" tab="概览与用法">
            <blockquote v-if="detail.presentation?.verse" class="plugin-verse"><p>{{ detail.presentation.verse }}</p><cite>《诗经 · {{ detail.presentation.poem }}》</cite></blockquote>
            <p class="detail-description">{{ detail.description }}</p>
            <section v-if="detail.presentation?.examples?.length" class="detail-section">
              <h3>这样使用</h3><div class="usage-examples"><div v-for="example in detail.presentation.examples" :key="example" class="usage-example"><span>{{ example }}</span><n-button text size="tiny" @click="copy(example)" :aria-label="`复制 ${example}`">复制</n-button></div></div>
            </section>
            <section class="detail-section" v-if="commands.length"><h3>快捷命令</h3><div class="command-chips"><code v-for="command in commands" :key="command">{{ command }}</code></div></section>
            <section class="detail-section"><h3>适用范围</h3><p class="section-note">{{ scopeDescription }}</p></section>
            <n-button type="primary" secondary @click="activeTab = 'config'">配置插件</n-button>
          </n-tab-pane>
          <n-tab-pane name="config" tab="配置">
            <div class="config-intro"><p>保存后自动应用。</p><span v-if="formDirty" class="unsaved">有未保存更改</span></div>
            <n-form label-placement="top" :show-feedback="false" :disabled="busy || detail.pending_restart" class="plugin-form">
              <n-form-item v-for="field in detail.config_fields || []" :key="field.key" :label="field.label">
                <n-select v-if="field.kind === 'choice'" v-model:value="configDraft[field.key]" :options="field.options || []" :aria-label="field.label" />
                <n-switch v-else-if="field.kind === 'boolean'" v-model:value="configDraft[field.key]" :aria-label="field.label" />
                <n-input-number v-else-if="field.kind === 'integer'" v-model:value="configDraft[field.key]" :precision="0" style="width:100%" :aria-label="field.label" />
                <div v-else class="field-input">
                  <n-input v-model:value="configDraft[field.key]" :type="field.secret ? 'password' : 'text'" :disabled="busy || detail.pending_restart || !!clearSecrets[field.key]" show-password-on="click" :placeholder="field.secret ? (field.has_value ? '已设置，留空保留原值' : '尚未设置') : ''" autocomplete="off" :aria-label="field.label" />
                  <div v-if="field.secret" class="secret-help"><span>{{ field.has_value ? '已安全保存' : '尚未保存' }}</span><n-checkbox v-if="field.has_value" v-model:checked="clearSecrets[field.key]">清除已保存的值</n-checkbox></div>
                </div>
              </n-form-item>
              <n-collapse class="advanced-config"><n-collapse-item title="调用限制与适用会话" name="limits">
                <div class="limit-fields"><n-form-item label="调用超时（秒）"><n-input-number v-model:value="timeoutSeconds" :min="0.1" :max="360" :step="1" /></n-form-item><n-form-item label="同时执行的任务数"><n-input-number v-model:value="runtimeDraft.max_concurrency" :min="1" :max="16" :precision="0" /></n-form-item></div>
                <n-form-item label="最大文字结果（KiB）"><n-input-number v-model:value="outputKiB" :min="1" :max="1024" /></n-form-item>
                <n-form-item label="适用会话"><n-select v-model:value="scopeMode" :options="scopeOptions" /></n-form-item>
                <n-input v-if="scopeMode === 'selected'" v-model:value="scopeText" type="textarea" placeholder="每行填写一个会话 ID，也可用逗号分隔" :autosize="{ minRows: 3, maxRows: 6 }" aria-label="允许使用插件的会话 ID" />
                <p class="field-help">限制仅作用于此插件；留空的指定列表将不允许任何会话调用。</p>
              </n-collapse-item></n-collapse>
            </n-form>
          </n-tab-pane>
          <n-tab-pane name="runtime" tab="调用状态">
            <p class="section-note">本次服务启动后的记录，配置更新会保留计数。</p>
            <dl class="runtime-grid"><div><dt>执行中</dt><dd>{{ detail.invocations?.active || 0 }}</dd></div><div><dt>调用成功</dt><dd>{{ detail.invocations?.succeeded || 0 }}</dd></div><div><dt>调用失败</dt><dd>{{ detail.invocations?.failed || 0 }}</dd></div><div><dt>最近耗时</dt><dd>{{ pluginDuration(detail.invocations?.last_duration_ms) }}</dd></div></dl>
            <n-alert v-if="detail.invocations?.last_error" type="warning" :bordered="false" class="plugin-alert" title="最近一次调用未完成">{{ detail.invocations.last_error }}</n-alert>
            <p v-else-if="detail.invocations?.last_status === 'succeeded'" class="runtime-success">最近一次调用已完成</p>
            <p class="section-note" v-else>尚无已完成的调用记录。</p>
            <div class="runtime-secondary"><span>未接纳 {{ detail.invocations?.rejected || 0 }} 次</span><span>已取消 {{ detail.invocations?.cancelled || 0 }} 次</span></div>
            <n-collapse class="technical-details"><n-collapse-item title="能力与存储信息" name="capabilities">
              <div v-for="skill in detail.skills || []" :key="skill.name" class="capability"><code>{{ skill.name }}</code><p>{{ skill.description }}</p></div>
              <div class="command-chips" v-if="detail.tools?.length"><code v-for="tool in detail.tools" :key="tool.name">{{ tool.name }}</code></div>
              <p v-if="!detail.skills?.length && !detail.tools?.length" class="section-note">启用后显示可调用的能力。</p>
              <p class="storage-path" v-if="detail.data_dir">数据目录 <code>{{ detail.data_dir }}</code></p>
              <p class="field-help">代码更新需要编译并更新云团服务；配置保存自动应用。</p>
            </n-collapse-item></n-collapse>
          </n-tab-pane>
        </n-tabs>
        <template #footer v-if="activeTab === 'config'"><div class="config-footer"><span aria-live="polite">{{ busy ? '正在应用，请稍候' : saved }}</span><div><n-button :disabled="busy || !formDirty" @click="resetDraft(detail)">撤销修改</n-button><n-button type="primary" :loading="busy" :disabled="busy || detail.pending_restart || (!formDirty && !detail.pending_reload)" @click="saveConfig">保存并应用</n-button></div></div></template>
      </n-drawer-content>
    </n-drawer>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useMessage } from 'naive-ui'
import { api } from '../api'
import { pluginDuration, pluginStatus } from '../plugin-ui'
import EmptyState from '../components/EmptyState.vue'
import PluginCard from '../components/PluginCard.vue'

const messages = useMessage()
const plugins = ref([])
const initialLoading = ref(true)
const loadError = ref('')
const actionError = ref('')
const selected = ref('')
const showDetail = ref(false)
const activeTab = ref('overview')
const query = ref('')
const filter = ref('all')
const pending = ref({})
const configDraft = ref({})
const runtimeDraft = ref({ timeout_ms: 30000, max_concurrency: 1, max_output_bytes: 65536 })
const clearSecrets = ref({})
const scopeMode = ref('all')
const scopeText = ref('')
const baseline = ref('')
const saved = ref('')
const viewportWidth = ref(window.innerWidth)
const drawerWidth = computed(() => Math.min(viewportWidth.value, 620))
const detail = computed(() => plugins.value.find(plugin => plugin.name === selected.value))
const busy = computed(() => !!pending.value[selected.value] || !!detail.value?.updating)
const status = computed(() => detail.value ? pluginStatus(detail.value, !!pending.value[selected.value]) : { tone: '', label: '' })
const readyCount = computed(() => plugins.value.filter(plugin => plugin.enabled && plugin.loaded).length)
const needsAttention = plugin => !plugin.updating && (plugin.error || plugin.pending_reload || plugin.pending_restart)
const filters = computed(() => [
  { value: 'all', label: '全部', count: plugins.value.length },
  { value: 'enabled', label: '已启用', count: plugins.value.filter(plugin => plugin.enabled).length },
  { value: 'attention', label: '待处理', count: plugins.value.filter(needsAttention).length },
])
const filtered = computed(() => plugins.value.filter(plugin => {
  if (filter.value === 'enabled' && !plugin.enabled) return false
  if (filter.value === 'attention' && !needsAttention(plugin)) return false
  const text = [plugin.name, plugin.display_name, plugin.description, plugin.presentation?.purpose].join(' ').toLowerCase()
  return text.includes(query.value.trim().toLowerCase())
}))
const commands = computed(() => [...new Set((detail.value?.skills || []).flatMap(skill => skill.commands || []))])
const scopeDescription = computed(() => {
  const allowed = detail.value?.settings?.allowed_chats
  return allowed == null ? '全部会话可用' : allowed.length ? `仅 ${allowed.length} 个指定会话可用` : '未开放给任何会话'
})
const scopeOptions = [{ value: 'all', label: '全部会话' }, { value: 'selected', label: '仅指定会话' }]
const timeoutSeconds = computed({ get: () => runtimeDraft.value.timeout_ms / 1000, set: value => { runtimeDraft.value.timeout_ms = value == null ? 0 : Math.round(value * 1000) } })
const outputKiB = computed({ get: () => runtimeDraft.value.max_output_bytes / 1024, set: value => { runtimeDraft.value.max_output_bytes = value == null ? 0 : Math.round(value * 1024) } })
const signature = () => JSON.stringify([configDraft.value, clearSecrets.value, runtimeDraft.value, scopeMode.value, scopeText.value])
const formDirty = computed(() => !!baseline.value && signature() !== baseline.value)

function resetDraft(plugin) {
  configDraft.value = Object.fromEntries((plugin.config_fields || []).map(field => [field.key, field.secret ? '' : (plugin.settings?.config?.[field.key] ?? field.default)]))
  runtimeDraft.value = { timeout_ms: plugin.settings?.timeout_ms ?? 30000, max_concurrency: plugin.settings?.max_concurrency ?? 1, max_output_bytes: plugin.settings?.max_output_bytes ?? 65536 }
  const allowed = plugin.settings?.allowed_chats
  scopeMode.value = allowed == null ? 'all' : 'selected'
  scopeText.value = (allowed || []).join('\n')
  clearSecrets.value = {}
  baseline.value = signature()
  saved.value = ''
}

function openDetail(plugin, tab = 'overview') {
  if (selected.value !== plugin.name || !formDirty.value) resetDraft(plugin)
  selected.value = plugin.name
  activeTab.value = tab
  actionError.value = ''
  showDetail.value = true
}

function settleChanges() {
  for (const [id, change] of Object.entries(pending.value)) {
    const plugin = plugins.value.find(plugin => plugin.name === id)
    if (change.phase !== 'waiting' || !plugin || plugin.updating) continue
    delete pending.value[id]
    if (plugin.error || plugin.pending_reload || plugin.pending_restart) {
      actionError.value = plugin.error || '设置尚未生效，请在详情中检查配置。'
    } else {
      const text = change.kind === 'config' ? '配置已保存并生效' : `${plugin.display_name || id}${plugin.enabled ? '已启用' : '已停用'}`
      if (change.kind === 'config' && selected.value === id) resetDraft(plugin)
      saved.value = text
      messages.success(text)
    }
  }
}

let disposed = false
let timer
let flight = null
let controller
async function load(fresh = false) {
  while (flight) {
    if (!fresh) return flight
    await flight
  }
  if (disposed) return
  controller = new AbortController()
  flight = (async () => {
    try {
      const { data } = await api.get('/plugins/list', { signal: controller.signal })
      if (disposed) return
      plugins.value = data.plugins || []
      loadError.value = ''
      settleChanges()
    } catch (error) {
      if (!disposed && error.code !== 'ERR_CANCELED') loadError.value = error.response?.data?.error || '插件状态暂时无法获取，请检查连接后重试。'
    } finally { initialLoading.value = false }
  })()
  try { await flight } finally { flight = null }
}

async function change(plugin, kind, path, body) {
  if (pending.value[plugin.name] || plugin.updating) return
  pending.value[plugin.name] = { kind, phase: 'sending' }
  actionError.value = ''
  saved.value = ''
  try {
    await api.post(path, body)
    if (disposed) return
    pending.value[plugin.name].phase = 'waiting'
    await load(true)
  } catch (error) {
    if (disposed) return
    delete pending.value[plugin.name]
    actionError.value = error.response?.data?.error || '操作未能确认，正在重新获取插件状态。'
    await load(true)
  }
}

async function toggle(plugin, enabled) {
  await change(plugin, 'toggle', '/plugins/toggle', { name: plugin.name, enabled })
}
async function saveConfig() {
  const plugin = detail.value
  if (!plugin || busy.value) return
  const settings = { ...plugin.settings, ...runtimeDraft.value, config: { ...plugin.settings?.config } }
  for (const field of plugin.config_fields || []) {
    const value = configDraft.value[field.key]
    if (field.secret) {
      delete settings.config[field.key]
      if (clearSecrets.value[field.key]) settings.config[field.key] = ''
      else if (value) settings.config[field.key] = value
    } else settings.config[field.key] = value
  }
  settings.allowed_chats = scopeMode.value === 'all' ? null : [...new Set(scopeText.value.split(/[\s,，]+/).filter(Boolean))]
  await change(plugin, 'config', '/plugins/config', { name: plugin.name, settings })
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); messages.success('已复制') }
  catch { messages.info('无法自动复制，请选中文字复制。') }
}
async function poll() {
  if (disposed) return
  if (!document.hidden) await load()
  if (!disposed) timer = setTimeout(poll, 2500)
}
function onVisible() { if (!document.hidden) void load() }
function onResize() { viewportWidth.value = window.innerWidth }
onMounted(() => { void poll(); document.addEventListener('visibilitychange', onVisible); window.addEventListener('resize', onResize) })
onUnmounted(() => { disposed = true; clearTimeout(timer); controller?.abort(); document.removeEventListener('visibilitychange', onVisible); window.removeEventListener('resize', onResize) })
</script>

<style scoped>
.plugins-page { max-width: 1280px; margin-inline: auto; }
:global(html.dark .plugins-page), :global(html.dark .plugin-drawer) { --yt-ink-3: #94a3b8; }
:global(html.dark .plugin-drawer .detail-state.success), :global(html.dark .plugin-drawer .runtime-success) { color: #74c7b8; }
.plugins-header { display: flex; align-items: center; justify-content: space-between; gap: 24px; padding: 3px 0 22px; }
.plugins-header h1 { margin: 0 0 8px; font-size: 25px; font-weight: 600; letter-spacing: .04em; color: var(--yt-ink-1); }
.plugins-header p { margin: 0; font-size: 13px; color: var(--yt-ink-3); }
.plugins-summary { display: flex; align-items: center; gap: 8px; flex-shrink: 0; font-size: 12px; color: var(--yt-ink-3); }
.summary-dot { width: 6px; height: 6px; border-radius: 50%; background: #258073; }
.summary-divider { height: 12px; width: 1px; background: var(--yt-card-border); margin: 0 5px; }
.plugins-toolbar { display: flex; justify-content: space-between; align-items: center; gap: 16px; margin-bottom: 20px; }
.plugin-filters { display: flex; gap: 4px; flex-shrink: 0; }
.plugin-filters button { border: 0; background: transparent; color: var(--yt-ink-3); font: inherit; font-size: 13px; padding: 8px 12px; border-radius: 8px; cursor: pointer; }
.plugin-filters button span { font-size: 11px; margin-left: 7px; opacity: .8; }
.plugin-filters button.selected { color: var(--yt-primary); background: var(--yt-active-bg); font-weight: 600; }
.plugin-filters button:focus-visible { outline: 2px solid var(--yt-primary); outline-offset: 1px; }
.plugin-search { max-width: 240px; }
.plugins-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 20px; }
.plugins-empty { padding: 50px 20px; background: var(--yt-surface); border: 1px dashed var(--yt-card-border); border-radius: 14px; }
.plugins-empty > p { text-align: center; color: var(--yt-ink-3); font-size: 12px; }
.plugins-footnote { margin: 22px 0 0; color: var(--yt-ink-3); font-size: 12px; }
.plugin-alert { margin: 12px 0; overflow-wrap: anywhere; }
.drawer-heading { display: flex; align-items: baseline; flex-wrap: wrap; gap: 12px; }
.drawer-name { font: 600 26px 'STSong', 'SimSun', 'Noto Serif SC', serif; letter-spacing: .08em; }
.drawer-purpose { color: var(--yt-ink-3); font-size: 12px; font-weight: 400; }
.drawer-status { display: flex; align-items: center; gap: 16px; margin: 10px 0 14px; color: var(--yt-ink-3); font-size: 12px; }
.detail-state { display: inline-flex; align-items: center; gap: 6px; }
.detail-state i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
.detail-state.success, .runtime-success { color: #258073; }
.detail-state.working { color: var(--yt-primary); }
.detail-state.warning { color: var(--yt-jk-num); }
.detail-state.error { color: var(--yt-danger); }
.plugin-verse { margin: 12px 0 22px; padding: 18px 20px; background: var(--yt-hover-bg); border-radius: 10px; }
.plugin-verse p { margin: 0 0 9px; font: 19px/1.6 'STSong', 'SimSun', 'Noto Serif SC', serif; color: var(--yt-ink-1); }
.plugin-verse cite { font-size: 11px; font-style: normal; color: var(--yt-ink-3); }
.detail-description { color: var(--yt-ink-2); font-size: 13px; line-height: 1.85; }
.detail-section { margin: 24px 0; }
h3 { margin: 0 0 12px; font-size: 13px; font-weight: 600; color: var(--yt-ink-1); }
.usage-examples { display: flex; flex-direction: column; border: 1px solid var(--yt-card-border); border-radius: 10px; overflow: hidden; }
.usage-example { display: flex; justify-content: space-between; gap: 12px; align-items: flex-start; padding: 12px 14px; font-size: 12px; line-height: 1.75; color: var(--yt-ink-2); }
.usage-example + .usage-example { border-top: 1px solid var(--yt-card-border); }
.usage-example span { overflow-wrap: anywhere; }
.usage-example .n-button { flex-shrink: 0; }
.command-chips { display: flex; gap: 8px; flex-wrap: wrap; }
.command-chips code { padding: 4px 8px; background: var(--yt-soft-bg); border-radius: 5px; font-size: 12px; overflow-wrap: anywhere; }
.section-note, .field-help { color: var(--yt-ink-3); font-size: 12px; line-height: 1.8; }
.config-intro { display: flex; justify-content: space-between; align-items: center; gap: 10px; margin-bottom: 16px; font-size: 12px; color: var(--yt-ink-3); }
.unsaved { color: var(--yt-jk-num); white-space: nowrap; }
.plugin-form :deep(.n-form-item) { margin-bottom: 18px; }
.field-input { width: 100%; display: flex; flex-direction: column; gap: 8px; }
.secret-help { display: flex; justify-content: space-between; gap: 12px; font-size: 11px; color: var(--yt-ink-3); }
.secret-help :deep(.n-checkbox__label) { font-size: 11px; }
.advanced-config { margin-top: 26px; padding-top: 18px; border-top: 1px solid var(--yt-card-border); }
.limit-fields { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.limit-fields :deep(.n-input-number) { width: 100%; }
.config-footer { display: flex; justify-content: space-between; align-items: center; gap: 12px; width: 100%; }
.config-footer > span { color: var(--yt-ink-3); font-size: 12px; }
.config-footer > div { display: flex; gap: 10px; flex-shrink: 0; }
.runtime-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 28px; margin: 24px 0; }
.runtime-grid > div { padding: 16px 0; border-bottom: 1px solid var(--yt-card-border); }
.runtime-grid dt { color: var(--yt-ink-3); font-size: 12px; margin-bottom: 8px; }
.runtime-grid dd { margin: 0; color: var(--yt-ink-1); font-size: 25px; font-weight: 500; font-variant-numeric: tabular-nums; }
.runtime-secondary { display: flex; gap: 24px; font-size: 12px; color: var(--yt-ink-3); margin: 20px 0 28px; }
.technical-details { padding-top: 18px; border-top: 1px solid var(--yt-card-border); }
.capability { margin: 10px 0 18px; }
.capability code { color: var(--yt-ink-1); font-size: 12px; overflow-wrap: anywhere; }
.capability p { color: var(--yt-ink-3); font-size: 12px; line-height: 1.8; }
.storage-path { color: var(--yt-ink-3); font-size: 11px; margin-top: 22px; }
.storage-path code { display: block; margin-top: 6px; overflow-wrap: anywhere; }
@media (max-width: 1100px) { .plugins-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@media (max-width: 680px) { .plugins-header { align-items: flex-start; flex-direction: column; gap: 14px; } .plugins-toolbar { align-items: stretch; flex-direction: column; gap: 12px; } .plugin-search { max-width: none; } .plugins-grid { grid-template-columns: 1fr; gap: 14px; } .plugin-filters button { padding: 8px 10px; } .config-footer { flex-wrap: wrap; } .config-footer > div { margin-left: auto; } }
</style>
