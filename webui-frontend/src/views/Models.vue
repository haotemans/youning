<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">模型服务</span>
        <span class="yt-page-sub mono">{{ providerList.length }} 个服务商</span>
      </div>
      <n-space :size="10">
        <span v-if="msg" :class="['save-msg', ok ? 'ok' : 'fail']">{{ msg }}</span>
        <n-button size="small" secondary @click="load">刷新</n-button>
        <n-button size="small" type="primary" :loading="saving" @click="save">保存配置</n-button>
      </n-space>
    </div>

    <n-card title="角色模型路由" size="small">
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
                <n-select v-model:value="roles[r].model" :options="modelOptions(r)" size="small"
                          filterable tag clearable placeholder="输入或选择模型 ID" />
              </div>
            </div>

            <div class="role-foot">
              <n-button size="tiny" secondary :loading="tests[r].state === 'loading'"
                        :disabled="!roles[r].provider" @click="testRole(r)">
                连通测试
              </n-button>
              <span v-if="tests[r].state === 'ok'" class="test-badge ok mono">
                {{ tests[r].latency }}ms
              </span>
              <span v-else-if="tests[r].state === 'fail'" class="test-badge fail" :title="tests[r].error">
                {{ tests[r].error || '连接失败' }}
              </span>
            </div>
          </div>
        </n-gi>
      </n-grid>
    </n-card>

    <n-card title="服务端点 (Providers)" size="small">
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
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'
import ProviderCard from '../components/ProviderCard.vue'
import YtIcon from '../components/YtIcon.vue'
import axios from 'axios'

/** @type {import('vue').Ref<Record<string, { base_url: string, modelsText: string, api_key_env: string, api_key_present: boolean, protocol: string }>>} */
const providers = ref({})
const roles = ref({
  decision: { provider: null, model: '' },
  bot_chat: { provider: null, model: '' },
  agent_exec: { provider: null, model: '' },
})
const roleNames = ['decision', 'bot_chat', 'agent_exec']
const roleIcons = { decision: 'trace', bot_chat: 'chat', agent_exec: 'tasks' }
const roleLabels = {
  decision: '发言决策',
  bot_chat: '人格对话',
  agent_exec: '任务执行',
}
const newName = ref('')
const saving = ref(false)
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

async function load() {
  try {
    const { data } = await api.get('/config')
    const ps = data.providers?.provider || {}
    /** @type {Record<string, { base_url: string, modelsText: string, api_key_env: string, api_key_present: boolean, protocol: string }>} */
    const next = {}
    for (const [name, p] of Object.entries(ps)) {
      next[name] = providers.value[name] || {
        base_url: p.base_url || '',
        modelsText: (p.models || []).join(', '),
        api_key_env: '',
        api_key_present: !!p.api_key_present,
        protocol: 'openai_chat',
      }
      next[name].api_key_present = !!p.api_key_present
      if (!next[name].protocol) next[name].protocol = 'openai_chat'
    }
    providers.value = next
    const rs = data.providers?.roles || {}
    for (const r of roleNames) {
      if (rs[r]) roles.value[r] = { provider: rs[r].provider, model: rs[r].model }
    }
  } catch { /* ignore */ }
}

onMounted(load)

async function testRole(r) {
  tests.value[r] = { state: 'loading', latency: null, model: '', error: '' }
  try {
    const { data } = await api.post('/llm/test', { role: r })
    tests.value[r] = {
      state: data.ok ? 'ok' : 'fail',
      latency: data.latency_ms ?? null,
      model: data.model || '',
      error: data.error || '',
    }
  } catch (e) {
    const err = axios.isAxiosError(e) ? (e.response?.data?.error || '请求失败') : '请求失败'
    tests.value[r] = { state: 'fail', latency: null, model: '', error: err }
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

async function save() {
  saving.value = true
  msg.value = ''
  try {
    const provider = {}
    for (const [name, p] of Object.entries(providers.value)) {
      provider[name] = {
        base_url: p.base_url,
        api_key_env: p.api_key_env,
        models: p.modelsText.split(',').map((s) => s.trim()).filter(Boolean),
      }
    }
    const rolesOut = {}
    for (const r of roleNames) {
      if (roles.value[r].provider) rolesOut[r] = { provider: roles.value[r].provider, model: roles.value[r].model }
    }
    const { data } = await api.post('/config', { providers: { provider, roles: rolesOut } })
    ok.value = true
    msg.value = `已保存（${(data.applied || []).join('、') || '已同步'}）`
  } catch (e) {
    ok.value = false
    msg.value = e.response?.data?.error || '保存失败'
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
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
