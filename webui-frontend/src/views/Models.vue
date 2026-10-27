<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">模型服务</span>
        <span class="yt-page-sub mono">{{ loaded ? `${providerList.length} 个服务商` : (loading ? '加载中…' : '配置未加载') }}</span>
      </div>
      <n-space :size="10">
        <span v-if="dirty" class="save-msg">有未保存更改</span>
        <span v-if="msg" :class="['save-msg', ok ? 'ok' : 'fail']">{{ msg }}</span>
        <n-button size="small" secondary :loading="loading" :disabled="saving || loading" @click="load">刷新</n-button>
        <n-button size="small" type="primary" :loading="saving" :disabled="!loaded || loading" @click="save">保存配置</n-button>
      </n-space>
    </div>

    <template v-if="!loaded">
      <div v-if="loading" class="models-loading" role="status">
        <n-spin size="small" />
        <span>正在加载模型配置…</span>
      </div>
      <n-alert v-else-if="msg" type="error" :bordered="false" role="alert">
        {{ msg }}
        <n-button size="small" secondary @click="load">重新加载</n-button>
      </n-alert>
      <n-skeleton v-if="loading" height="320px" :sharp="false" />
    </template>

    <n-card v-if="loaded" title="角色模型路由" size="small">
      <template #header-extra>
        <n-button size="small" secondary :disabled="!loaded || loading || saving" @click="applyRecommendedReasoning">
          决策 low / 回复 high
        </n-button>
      </template>
      <n-grid cols="1 m:3" :x-gap="16" :y-gap="16" responsive="screen">
        <n-gi v-for="r in roleNames" :key="r">
          <div class="role-card" :class="{ bound: roles[r].provider }">
            <div class="role-head">
              <div class="role-icon">
                <yt-icon :name="roleIcons[r]" :size="16" />
              </div>
              <div class="role-meta">
                <span class="role-name mono">{{ r }}</span>
                <span class="role-label">{{ roleLabels[r] }}</span>
              </div>
              <n-tag size="tiny" :bordered="false" :type="roles[r].provider ? 'success' : 'default'">
                {{ roles[r].provider ? '已绑定' : '未绑定' }}
              </n-tag>
            </div>

            <div class="role-fields">
              <div class="field-item">
                <label class="field-label">服务商</label>
                <n-select v-model:value="roles[r].provider" :options="providerOptions" size="small"
                          placeholder="选择 Provider" clearable />
              </div>
              <div class="field-item">
                <label class="field-label">模型 ID</label>
                <n-select v-model:value="roles[r].model" :options="roleModelOptions[r]" size="small"
                          filterable tag clearable placeholder="输入或选择模型 ID" />
              </div>
              <div class="field-item">
                <label class="field-label">思考强度</label>
                <n-select v-model:value="roles[r].reasoning_effort" :options="reasoningOptions" size="small"
                          placeholder="服务商默认" clearable />
                <span class="field-label">{{ reasoningHints[r] }}；实际效果取决于模型和服务商。</span>
              </div>
            </div>

            <div class="role-foot">
              <n-button size="tiny" secondary :loading="tests[r].state === 'loading'"
                        :disabled="!roles[r].provider || !roles[r].model || !loaded || loading || saving" @click="testRole(r)">
                保存并测试
              </n-button>
              <span v-if="tests[r].state === 'ok'" class="test-badge ok mono">
                {{ tests[r].model }}{{ roles[r].reasoning_effort ? ` · ${roles[r].reasoning_effort}` : '' }} · {{ tests[r].latency }}ms
              </span>
              <span v-else-if="tests[r].state === 'fail'" class="test-badge fail" :title="tests[r].error">
                {{ tests[r].error || '连接失败' }}
              </span>
            </div>
          </div>
        </n-gi>
      </n-grid>
    </n-card>

    <n-card v-if="loaded" title="服务端点 (Providers)" size="small">
      <template #header-extra>
        <div class="header-actions">
          <n-input v-model:value="newName" size="small" placeholder="输入新服务商标识"
                   :status="newNameError ? 'error' : undefined"
                   style="width: 190px" @keyup.enter="addProvider" />
          <n-button size="small" type="primary" secondary @click="addProvider">新增服务商</n-button>
        </div>
      </template>

      <n-alert v-if="newNameError" type="warning" :bordered="false" size="small" style="margin-bottom: 12px">
        {{ newNameError }}
      </n-alert>

      <div v-if="providerList.length" class="provider-list">
        <provider-card v-for="item in providerList" :key="item.name" :name="item.name" :p="item.p"
                       :usage="providerUsage(item.name)" @delete="delProvider(item.name)" />
      </div>
      <empty-state v-else title="尚未添加模型服务商" />
    </n-card>
  </div>
</template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'
import ProviderCard from '../components/ProviderCard.vue'
import YtIcon from '../components/YtIcon.vue'
import axios from 'axios'

/** @type {import('vue').Ref<Record<string, { base_url: string, modelsText: string, api_key_env: string, api_key_present: boolean, protocol: string }>>} */
const providers = ref({})
const roles = ref({
  decision: { provider: null, model: '', reasoning_effort: null },
  bot_chat: { provider: null, model: '', reasoning_effort: null },
  agent_exec: { provider: null, model: '', reasoning_effort: null },
})
const reasoningOptions = [
  { label: 'low · 轻量思考', value: 'low' },
  { label: 'medium · 平衡', value: 'medium' },
  { label: 'high · 深入思考', value: 'high' },
]
const reasoningHints = {
  decision: '建议 low，优先快速判断是否回应及使用什么能力',
  bot_chat: '建议 high，为组织回答和核实依据留更多思考空间',
  agent_exec: '按任务复杂度选择，留空沿用服务商设置',
}
function applyRecommendedReasoning() {
  roles.value.decision.reasoning_effort = 'low'
  roles.value.bot_chat.reasoning_effort = 'high'
}
const roleNames = ['decision', 'bot_chat', 'agent_exec']
const roleIcons = { decision: 'trace', bot_chat: 'chat', agent_exec: 'tasks' }
const roleLabels = {
  decision: '发言决策',
  bot_chat: '人格对话',
  agent_exec: '任务执行',
}
const newName = ref('')
const saving = ref(false)
const loading = ref(false)
const loaded = ref(false)
const savedSignature = ref('')
const msg = ref('')
const ok = ref(false)
/** @type {import('vue').Ref<Record<string, { state: string, latency: number|null, model: string, error: string }>>} */
const tests = ref({
  decision: { state: 'idle', latency: null, model: '', error: '' },
  bot_chat: { state: 'idle', latency: null, model: '', error: '' },
  agent_exec: { state: 'idle', latency: null, model: '', error: '' },
})

const providerOptions = computed(() => Object.keys(providers.value).map((n) => ({ label: n, value: n })))
const providerList = computed(() => Object.entries(providers.value).map(([name, p]) => ({ name, p })))
const dirty = computed(() => loaded.value && JSON.stringify(configPayload()) !== savedSignature.value)
const testEpoch = { decision: 0, bot_chat: 0, agent_exec: 0 }
const lifecycle = new AbortController()
let disposed = false
onUnmounted(() => { disposed = true; lifecycle.abort() })

watch([providers, roles], () => {
  msg.value = ''
  for (const r of roleNames) {
    testEpoch[r]++
    tests.value[r] = { state: 'idle', latency: null, model: '', error: '' }
  }
}, { deep: true, flush: 'sync' })

function providerUsage(name) {
  return roleNames.filter((r) => roles.value[r].provider === name)
}

function modelOptions(role) {
  const provider = providers.value[roles.value[role].provider]
  const models = (provider?.modelsText || '').split(',').map((s) => s.trim()).filter(Boolean)
  const current = roles.value[role].model?.trim()
  if (current && !models.includes(current)) models.unshift(current)
  return [...new Set(models)].map((model) => ({ label: model, value: model }))
}
const roleModelOptions = computed(() => Object.fromEntries(roleNames.map(r => [r, modelOptions(r)])))

function applyConfig(data) {
  const ps = data.providers?.provider || {}
  /** @type {Record<string, { base_url: string, modelsText: string, api_key_env: string, api_key_present: boolean, protocol: string }>} */
  const next = {}
  for (const [name, p] of Object.entries(ps)) {
    next[name] = {
      base_url: p.base_url || '',
      modelsText: (p.models || []).join(', '),
      api_key_env: '',
      api_key_present: !!p.api_key_present,
      protocol: 'openai_chat',
    }
  }
  providers.value = next
  const rs = data.providers?.roles || {}
  for (const r of roleNames) {
    roles.value[r] = rs[r]
      ? { provider: rs[r].provider, model: rs[r].model, reasoning_effort: rs[r].reasoning_effort ?? null }
      : { provider: null, model: '', reasoning_effort: null }
  }
  savedSignature.value = JSON.stringify(configPayload())
  loaded.value = true
}

async function load() {
  if (loading.value) return
  loading.value = true
  try {
    const { data } = await api.get('/config', { signal: lifecycle.signal })
    if (disposed) return
    applyConfig(data)
  } catch (e) {
    if (disposed) return
    ok.value = false
    msg.value = axios.isAxiosError(e) ? (e.response?.data?.error || '加载配置失败') : '加载配置失败'
  } finally {
    loading.value = false
  }
}

onMounted(load)

async function testRole(r) {
  if (saving.value || tests.value[r].state === 'loading') return
  if (!await save()) {
    tests.value[r] = { state: 'fail', latency: null, model: roles.value[r].model, error: msg.value || '配置尚未保存，未发起测试' }
    return
  }
  const model = roles.value[r].model
  const effort = roles.value[r].reasoning_effort
  const epoch = ++testEpoch[r]
  tests.value[r] = { state: 'loading', latency: null, model: '', error: '' }
  try {
    const { data } = await api.post('/llm/test', { role: r }, { timeout: 65000 })
    if (epoch !== testEpoch[r]) return
    if (data.ok && data.model !== model) throw new Error(`实际测试模型 ${data.model} 与所选模型 ${model} 不一致`)
    if (data.ok && (data.reasoning_effort ?? null) !== effort) throw new Error('实际测试的思考强度与所选配置不一致')
    tests.value[r] = {
      state: data.ok ? 'ok' : 'fail',
      latency: data.latency_ms ?? null,
      model: data.model || '',
      error: data.error || '',
    }
  } catch (e) {
    if (epoch !== testEpoch[r]) return
    const err = axios.isAxiosError(e) ? (e.response?.data?.error || '请求失败') : (e.message || '请求失败')
    tests.value[r] = { state: 'fail', latency: null, model, error: `测试 ${model}：${err}` }
  }
}

const newNameError = ref('')
function addProvider() {
  const n = newName.value.trim()
  if (!n) {
    newNameError.value = '请先输入服务商标识名称（如 openai 或 deepseek）'
    return
  }
  if (providers.value[n]) {
    newNameError.value = `服务商 "${n}" 已存在`
    return
  }
  newNameError.value = ''
  providers.value[n] = { base_url: '', modelsText: '', api_key_env: '', api_key_present: false, protocol: 'openai_chat' }
  newName.value = ''
}

function delProvider(n) {
  delete providers.value[n]
  for (const r of roleNames) {
    if (roles.value[r].provider === n) roles.value[r].provider = null
  }
}

function configPayload() {
  const provider = {}
  for (const name of Object.keys(providers.value).sort()) {
    const p = providers.value[name]
    provider[name] = {
      base_url: p.base_url,
      api_key_env: p.api_key_env,
      models: p.modelsText.split(',').map((s) => s.trim()).filter(Boolean),
    }
  }
  const rolesOut = {}
  for (const r of roleNames) {
    if (roles.value[r].provider) {
      rolesOut[r] = { provider: roles.value[r].provider, model: roles.value[r].model }
      if (roles.value[r].reasoning_effort) rolesOut[r].reasoning_effort = roles.value[r].reasoning_effort
    }
  }
  return { provider, roles: rolesOut }
}

function routingSignature(config) {
  const provider = {}
  for (const name of Object.keys(config.provider || {}).sort()) {
    const p = config.provider[name]
    provider[name] = { base_url: p.base_url, models: p.models || [] }
  }
  const bindings = {}
  for (const r of roleNames) {
    const bind = config.roles?.[r]
    if (bind) {
      bindings[r] = { provider: bind.provider, model: bind.model }
      if (bind.reasoning_effort) bindings[r].reasoning_effort = bind.reasoning_effort
    }
  }
  return JSON.stringify({ provider, roles: bindings })
}

async function save() {
  if (!loaded.value || loading.value || saving.value) return false
  for (const r of roleNames) {
    if (roles.value[r].provider && !roles.value[r].model?.trim()) {
      ok.value = false
      msg.value = `请为 ${roleLabels[r]} 选择模型`
      return false
    }
  }
  const requested = configPayload()
  const signature = JSON.stringify(requested)
  saving.value = true
  msg.value = ''
  try {
    const { data } = await api.post('/config', { providers: requested })
    const { data: saved } = await api.get('/config')
    if (routingSignature(requested) !== routingSignature(saved.providers || {})) {
      throw new Error('保存后回读的模型配置不一致，请重试')
    }
    if (JSON.stringify(configPayload()) !== signature) {
      ok.value = false
      msg.value = '提交的配置已保存，编辑中的新更改请再次保存'
      return false
    }
    applyConfig(saved)
    ok.value = true
    msg.value = `已保存（${(data.applied || []).join('、') || '已同步'}）`
    return true
  } catch (e) {
    ok.value = false
    msg.value = axios.isAxiosError(e) ? (e.response?.data?.error || '保存失败') : (e.message || '保存失败')
    return false
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.models-loading {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 18px;
  color: var(--yt-ink-2);
  background: var(--yt-soft-bg);
  border-radius: 10px;
}
.role-card {
  border: 1px solid var(--yt-card-border);
  border-radius: 10px;
  padding: 16px;
  background: var(--yt-elevated);
  display: flex;
  flex-direction: column;
  gap: 14px;
  transition: border-color 0.15s ease;
}
.role-card.bound {
  background: var(--yt-surface);
  border-color: var(--yt-card-border-hover);
}
.role-head {
  display: flex;
  align-items: center;
  gap: 10px;
}
.role-icon {
  width: 32px;
  height: 32px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--yt-active-bg);
  color: var(--yt-primary);
  flex: none;
}
.role-meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.role-name {
  font-size: 13.5px;
  font-weight: 600;
  color: var(--yt-ink-1);
}
.role-label {
  font-size: 12px;
  color: var(--yt-ink-3);
}
.role-fields {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.field-label {
  display: block;
  font-size: 12px;
  font-weight: 500;
  color: var(--yt-ink-2);
  margin-bottom: 4px;
}
.role-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding-top: 10px;
  border-top: 1px solid var(--yt-card-border);
}
.test-badge {
  font-size: 12px;
  font-weight: 500;
  padding: 2px 8px;
  border-radius: 6px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.test-badge.ok {
  background: var(--yt-ok-soft);
  color: var(--yt-ok);
}
.test-badge.fail {
  background: var(--yt-danger-soft);
  color: var(--yt-danger);
}
.save-msg {
  font-size: 12.5px;
  font-weight: 500;
}
.save-msg.ok { color: var(--yt-ok); }
.save-msg.fail { color: var(--yt-danger); }
.provider-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
