<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">MCP 服务</span>
        <span class="yt-page-sub mono">{{ servers.length }} 个服务</span>
      </div>
      <n-space align="center" :size="10">
        <span v-if="saveNotice" :class="['save-msg', saveOk ? 'ok' : 'fail']">{{ saveNotice }}</span>
        <n-button size="small" secondary @click="load">刷新</n-button>
        <n-button size="small" secondary @click="openAdd">新增服务</n-button>
        <n-button size="small" type="primary" :disabled="!dirtyFlag" @click="saveAll">保存配置</n-button>
      </n-space>
    </div>

    <n-alert v-if="dirtyFlag" type="warning" :bordered="false">
      MCP 配置存在未保存修改，点击右上角「保存配置」写入配置文件，重启服务后生效。
    </n-alert>

    <div v-if="servers.length" class="cap-grid">
      <McpServerCard
        v-for="s in servers"
        :key="s.name"
        :server="s"
        :dirty="dirtyFlag"
        @toggle="(sv, v) => { sv.enabled = v; markDirty() }"
        @open="openEdit"
      />
    </div>
    <n-card v-else size="small">
      <empty-state title="暂未配置 MCP 服务" />
    </n-card>

    <n-modal v-model:show="showEdit" preset="card" :title="editForm.isNew ? '新增 MCP 服务' : `配置 ${editForm.name}`" style="max-width: 560px">
      <div class="form">
        <div class="form-row">
          <span class="k">服务标识</span>
          <n-input size="small" v-model:value="editForm.name" :disabled="!editForm.isNew" placeholder="如 filesystem 或 fetch" />
        </div>
        <div class="form-row">
          <span class="k">启动命令</span>
          <n-input size="small" v-model:value="editForm.command" placeholder="如 npx 或 uvx" />
        </div>
        <div class="form-row">
          <span class="k">命令参数</span>
          <n-input size="small" v-model:value="editForm.argsText" placeholder="空格分隔参数" />
        </div>
        <div class="form-row">
          <span class="k">环境变量</span>
          <n-input size="small" type="textarea" :autosize="{ minRows: 2, maxRows: 5 }"
                   v-model:value="editForm.envText" placeholder="每行一条 KEY=VALUE" />
        </div>
        <div class="form-row">
          <span class="k">启用状态</span>
          <n-switch v-model:value="editForm.enabled" size="small" />
        </div>
      </div>

      <template #footer>
        <div style="display: flex; gap: 8px; justify-content: space-between; width: 100%">
          <n-button v-if="!editForm.isNew" size="small" type="error" quaternary @click="onDelete">删除服务</n-button>
          <span v-else />
          <div style="display: flex; gap: 8px">
            <n-button size="small" @click="showEdit = false">取消</n-button>
            <n-button size="small" type="primary" :disabled="!formValid" @click="onSubmit">
              {{ editForm.isNew ? '添加' : '确认修改' }}
            </n-button>
          </div>
        </div>
      </template>
    </n-modal>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'
import McpServerCard from '../components/McpServerCard.vue'

const servers = ref([])
const dirtyFlag = ref(false)
const saveNotice = ref('')
const saveOk = ref(true)

const showEdit = ref(false)
const editForm = ref(emptyForm())
const editingIndex = ref(-1)

function emptyForm() {
  return { isNew: true, name: '', command: '', argsText: '', envText: '', enabled: true }
}

const formValid = computed(() => editForm.value.name.trim() && editForm.value.command.trim())

function markDirty() {
  dirtyFlag.value = true
  saveNotice.value = ''
}

async function load() {
  const { data } = await api.get('/mcp/list')
  servers.value = (data.servers || []).map(s => ({ ...s, env: {} }))
  dirtyFlag.value = false
}

function openAdd() {
  editForm.value = emptyForm()
  editingIndex.value = -1
  showEdit.value = true
}

function openEdit(s) {
  const idx = servers.value.findIndex(x => x.name === s.name)
  if (idx < 0) return
  editingIndex.value = idx
  const envText = (s.env_keys || []).map(k => `${k}=`).join('\n')
  editForm.value = {
    isNew: false,
    name: s.name,
    command: s.command,
    argsText: (s.args || []).join(' '),
    envText,
    enabled: s.enabled,
  }
  showEdit.value = true
}

function onSubmit() {
  const f = editForm.value
  const args = f.argsText.split(/\s+/).filter(x => x)
  const env = {}
  for (const line of f.envText.split('\n')) {
    const t = line.trim()
    if (!t || t.startsWith('#')) continue
    const eq = t.indexOf('=')
    if (eq < 0) continue
    env[t.slice(0, eq)] = t.slice(eq + 1)
  }
  const next = {
    name: f.name.trim(),
    command: f.command.trim(),
    args,
    env,
    env_keys: Object.keys(env),
    enabled: f.enabled,
    running: false,
    tools: [],
  }
  if (f.isNew) servers.value.push(next)
  else servers.value.splice(editingIndex.value, 1, next)
  dirtyFlag.value = true
  saveNotice.value = ''
  showEdit.value = false
}

function onDelete() {
  if (editingIndex.value >= 0) {
    servers.value.splice(editingIndex.value, 1)
    dirtyFlag.value = true
    saveNotice.value = ''
  }
  showEdit.value = false
}

async function saveAll() {
  saveNotice.value = ''
  try {
    const body = servers.value.map(s => ({
      name: s.name,
      command: s.command,
      args: s.args,
      env: s.env || {},
      enabled: s.enabled,
    }))
    const { data } = await api.post('/mcp/save', { servers: body })
    if (data.ok) {
      dirtyFlag.value = false
      saveOk.value = true
      saveNotice.value = '已保存（重启服务后生效）'
    }
  } catch (e) {
    saveOk.value = false
    saveNotice.value = e?.response?.data?.error || e.message || '保存失败'
  }
}

onMounted(load)
</script>

<style scoped>
.form { display: flex; flex-direction: column; gap: 12px; }
.form-row { display: flex; align-items: center; gap: 12px; }
.form-row .k {
  flex: none;
  width: 90px;
  font-size: 12.5px;
  font-weight: 500;
  color: var(--yt-ink-2);
}
.save-msg {
  font-size: 12.5px;
  font-weight: 500;
}
.save-msg.ok { color: var(--yt-ok); }
.save-msg.fail { color: var(--yt-danger); }
</style>
