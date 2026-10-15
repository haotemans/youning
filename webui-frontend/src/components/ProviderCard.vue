<template>
  <div class="pc-wrap">
    <div class="pc-row" :class="{ active: expanded }" @click="expanded = !expanded">
      <div class="pc-icon">
        <span class="pc-status-dot" :class="{ ok: p.api_key_present }" />
      </div>
      <div class="pc-info">
        <div class="pc-headline">
          <span class="pc-name mono">{{ name }}</span>
          <n-tag size="tiny" :bordered="false" class="pc-tint">
            {{ p.protocol === 'openai_chat' ? 'OpenAI Chat' : p.protocol }}
          </n-tag>
          <n-tag v-for="r in usage" :key="r" size="tiny" type="info" :bordered="false">{{ r }}</n-tag>
        </div>
        <div class="pc-sub mono">{{ p.base_url || '未配置端点地址' }}</div>
      </div>
      <div class="pc-actions" @click.stop>
        <n-button size="small" secondary @click="expanded = !expanded">
          {{ expanded ? '收起' : '编辑' }}
        </n-button>
        <n-popconfirm @positive-click="$emit('delete')">
          <template #trigger>
            <n-button size="small" quaternary type="error">删除</n-button>
          </template>
          确认删除服务商 {{ name }}？
        </n-popconfirm>
      </div>
    </div>

    <n-collapse-transition :show="expanded">
      <div class="pc-edit">
        <div class="edit-grid">
          <div class="field-block">
            <label class="field-label">标识名称</label>
            <n-input :value="name" disabled size="small" />
          </div>
          <div class="field-block">
            <label class="field-label">接口协议</label>
            <n-select v-model:value="p.protocol" :options="protocolOptions" size="small" />
          </div>
          <div class="field-block">
            <label class="field-label">Base URL</label>
            <n-input v-model:value="p.base_url" placeholder="https://api.example.com/v1" size="small" />
          </div>
          <div class="field-block">
            <label class="field-label">API Key / 环境变量</label>
            <n-input v-model:value="p.api_key_env" type="password" show-password-on="click"
                     :placeholder="p.api_key_present ? '已配置，留空保持不变' : 'sk-... 或环境变量名'" size="small" />
          </div>
        </div>

        <div class="models-block">
          <div class="models-head">
            <span class="models-title">可用模型（{{ selectedModels.length }}）</span>
            <n-space size="small">
              <n-input v-model:value="newModel" size="small" placeholder="输入模型 ID" style="width: 180px" @keyup.enter="addModel" />
              <n-button size="small" secondary @click.stop="addModel">添加</n-button>
              <n-button size="small" secondary :loading="fetching" :disabled="!p.base_url" @click.stop="fetchModels">
                探测远端模型
              </n-button>
            </n-space>
          </div>

          <div v-if="fetchError" class="fetch-msg fail">{{ fetchError }}</div>
          <div v-else-if="fetchOk && availableModels.length" class="models-cloud">
            <span
              v-for="m in availableModels"
              :key="m"
              class="model-chip mono"
              :class="{ on: isSelected(m) }"
              @click.stop="toggleModel(m)"
            >{{ m }}</span>
          </div>

          <div v-if="selectedModels.length" class="sel-cloud">
            <span v-for="m in selectedModels" :key="m" class="model-chip sel mono" @click.stop="removeModel(m)">
              {{ m }} <span class="x">×</span>
            </span>
          </div>
        </div>
      </div>
    </n-collapse-transition>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue'
import axios from 'axios'
import { api } from '../api'

const props = defineProps({
  name: { type: String, required: true },
  p: { type: /** @type {import('vue').PropType<{ base_url: string, modelsText: string, api_key_env: string, api_key_present: boolean, protocol: string }>} */ (Object), required: true },
  usage: { type: /** @type {import('vue').PropType<string[]>} */ (Array), default: () => [] },
})
defineEmits(['delete'])

const expanded = ref(false)
const fetching = ref(false)
const fetchError = ref('')
const fetchOk = ref(false)
const newModel = ref('')
const availableModels = ref([])

const protocolOptions = [
  { label: 'OpenAI Chat Completions', value: 'openai_chat' },
]

const selectedModels = computed(() =>
  props.p.modelsText.split(',').map(s => s.trim()).filter(Boolean)
)
function isSelected(m) { return selectedModels.value.includes(m) }
function toggleModel(m) {
  const cur = new Set(selectedModels.value)
  if (cur.has(m)) cur.delete(m); else cur.add(m)
  props.p.modelsText = [...cur].join(', ')
}
function addModel() {
  const m = newModel.value.trim()
  if (!m) return
  if (!selectedModels.value.includes(m)) {
    props.p.modelsText = [...selectedModels.value, m].join(', ')
  }
  newModel.value = ''
}
function removeModel(m) {
  props.p.modelsText = selectedModels.value.filter(x => x !== m).join(', ')
}

async function fetchModels() {
  fetching.value = true
  fetchError.value = ''
  fetchOk.value = false
  try {
    const { data } = props.p.api_key_env
      ? await api.post('/llm/models/probe', {
          base_url: props.p.base_url,
          api_key_env: props.p.api_key_env,
        })
      : props.p.api_key_present
        ? await api.get('/llm/models', { params: { provider: props.name } })
        : await api.post('/llm/models/probe', { base_url: props.p.base_url, api_key_env: '' })
    availableModels.value = data.models || []
    fetchOk.value = true
    if (!availableModels.value.length) fetchError.value = '远端返回空模型列表'
  } catch (e) {
    fetchError.value = axios.isAxiosError(e) ? (e.response?.data?.error || '探测失败') : '探测失败'
    availableModels.value = []
  } finally {
    fetching.value = false
  }
}
</script>

<style scoped>
.pc-wrap {
  border: 1px solid var(--yt-card-border);
  border-radius: 10px;
  background: var(--yt-surface);
  overflow: hidden;
  transition: border-color 0.15s ease;
}
.pc-wrap:hover {
  border-color: var(--yt-card-border-hover);
}
.pc-row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 12px 16px;
  cursor: pointer;
  user-select: none;
}
.pc-row:hover {
  background: var(--yt-elevated);
}
.pc-row.active {
  border-bottom: 1px solid var(--yt-card-border);
  background: var(--yt-soft-bg);
}
.pc-icon {
  width: 28px;
  height: 28px;
  flex: none;
  border-radius: 7px;
  background: var(--yt-soft-bg);
  border: 1px solid var(--yt-card-border);
  display: flex;
  align-items: center;
  justify-content: center;
}
.pc-status-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--yt-ink-3);
}
.pc-status-dot.ok {
  background: var(--yt-ok);
}
.pc-info { flex: 1; min-width: 0; }
.pc-headline {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.pc-name {
  font-weight: 600;
  font-size: 14px;
  color: var(--yt-ink-1);
}
.pc-tint {
  background: var(--yt-active-bg);
  color: var(--yt-primary);
}
.pc-sub {
  margin-top: 2px;
  font-size: 12px;
  color: var(--yt-ink-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.pc-actions {
  display: flex;
  gap: 6px;
  flex: none;
}
.pc-edit {
  padding: 16px 18px;
  background: var(--yt-elevated);
}
.edit-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px 16px;
  margin-bottom: 14px;
}
.field-label {
  display: block;
  font-size: 12px;
  font-weight: 500;
  color: var(--yt-ink-2);
  margin-bottom: 4px;
}
.models-block {
  padding-top: 12px;
  border-top: 1px solid var(--yt-card-border);
}
.models-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  margin-bottom: 10px;
  flex-wrap: wrap;
}
.models-title {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--yt-ink-2);
}
.fetch-msg {
  padding: 6px 10px;
  border-radius: 6px;
  font-size: 12px;
  margin-bottom: 8px;
}
.fetch-msg.fail { background: var(--yt-danger-soft); color: var(--yt-danger); }
.models-cloud {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  padding: 10px;
  background: var(--yt-surface);
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  max-height: 150px;
  overflow-y: auto;
  margin-bottom: 10px;
}
.sel-cloud {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.model-chip {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 9px;
  background: var(--yt-code-bg);
  border: 1px solid var(--yt-card-border);
  border-radius: 6px;
  font-size: 12px;
  color: var(--yt-ink-2);
  cursor: pointer;
  user-select: none;
}
.model-chip:hover {
  border-color: var(--yt-primary);
  color: var(--yt-primary);
}
.model-chip.on,
.model-chip.sel {
  background: var(--yt-primary);
  color: #fff;
  border-color: var(--yt-primary);
}
.model-chip.sel .x {
  margin-left: 2px;
  opacity: 0.75;
}
@media (max-width: 720px) {
  .edit-grid { grid-template-columns: 1fr; }
}
</style>
