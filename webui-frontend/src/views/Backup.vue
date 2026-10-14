<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">数据备份</span>
        <span class="yt-page-sub mono">{{ backups.length }} 个归档</span>
      </div>
      <n-space align="center" :size="10">
        <span v-if="lastRun" :class="['run-msg', lastRunOk ? 'ok' : 'fail']">{{ lastRun }}</span>
        <n-button size="small" secondary @click="reload">刷新</n-button>
        <n-button size="small" type="primary" :loading="running" @click="runNow">
          {{ running ? '正在备份…' : '立即备份' }}
        </n-button>
      </n-space>
    </div>

    <n-grid cols="1 s:2 l:4" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" :style="{ background: cfg.enabled && cfg.repo_url ? 'var(--yt-ok)' : 'var(--yt-warning)' }" />
            自动备份状态
          </div>
          <div class="stat-foot">
            <span class="stat-num">{{ cfg.enabled && cfg.repo_url ? '已启用' : '未启用' }}</span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-primary)" />
            本地归档数量
          </div>
          <div class="stat-foot">
            <span class="stat-num mono">{{ backups.length }}</span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-violet)" />
            归档总体积
          </div>
          <div class="stat-foot">
            <span class="stat-num mono">{{ totalSize }}</span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-ok)" />
            最近备份时间
          </div>
          <div class="stat-foot">
            <span class="stat-num mono" style="font-size: 20px">{{ lastBackupAgo }}</span>
          </div>
        </n-card>
      </n-gi>
    </n-grid>

    <n-grid cols="1 l:2" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card title="备份策略与远端同步" size="small">
          <n-form label-placement="left" label-width="110" :show-feedback="false" class="clean-form">
            <n-form-item label="每日自动备份">
              <n-switch v-model:value="cfg.enabled" />
            </n-form-item>
            <n-form-item label="Git 仓库地址">
              <n-input v-model:value="cfg.repo_url" placeholder="https://github.com/user/yuantuan-backup.git" />
            </n-form-item>
            <n-form-item label="PAT 环境变量">
              <n-input v-model:value="cfg.pat_env" placeholder="YUANTUAN_BACKUP_PAT" />
            </n-form-item>
            <n-form-item label="执行时刻">
              <n-input v-model:value="cfg.daily_time" placeholder="03:00" style="width: 150px" />
            </n-form-item>
            <n-form-item label="保留天数">
              <n-input-number v-model:value="cfg.keep_days" :min="1" :max="90" style="width: 150px" />
            </n-form-item>
          </n-form>

          <div class="form-actions">
            <n-button type="primary" size="small" secondary :loading="saving" @click="save">保存备份策略</n-button>
            <span v-if="saveMsg" :class="['run-msg', saveOk ? 'ok' : 'fail']">{{ saveMsg }}</span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card title="历史归档列表" size="small">
          <n-data-table
            v-if="backups.length"
            :columns="cols"
            :data="backups"
            size="small"
            :pagination="{ pageSize: 8 }"
          />
          <empty-state v-else title="暂无历史备份文件" />
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup>
import { computed, h, onMounted, ref } from 'vue'
import { NButton, NPopconfirm } from 'naive-ui'
import { api } from '../api'
import { fmtTs } from '../fmt'
import EmptyState from '../components/EmptyState.vue'

const cfg = ref({ enabled: false, repo_url: '', pat_env: 'YUANTUAN_BACKUP_PAT', daily_time: '03:00', keep_days: 7 })
const backups = ref([])
const running = ref(false)
const saving = ref(false)
const lastRun = ref('')
const lastRunOk = ref(false)
const saveMsg = ref('')
const saveOk = ref(false)

const totalSize = computed(() => {
  const sum = backups.value.reduce((a, b) => a + (b.size_bytes || 0), 0)
  return fmtSize(sum)
})
const lastBackupAgo = computed(() => {
  const t = backups.value[0]?.created_at
  if (!t) return '—'
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - t)
  if (diff < 60) return `${diff}s 前`
  if (diff < 3600) return `${Math.floor(diff / 60)}m 前`
  if (diff < 86400) return `${Math.floor(diff / 3600)}h 前`
  return `${Math.floor(diff / 86400)}d 前`
})

const cols = [
  { title: '归档文件', key: 'name', render: (r) => h('code', { style: 'font-size: 12px' }, r.name) },
  { title: '大小', key: 'size_bytes', width: 95, render: (r) => fmtSize(r.size_bytes) },
  { title: '创建时间', key: 'created_at', width: 160, render: (r) => fmtTs(r.created_at) },
  {
    title: '操作',
    key: 'op',
    width: 130,
    render: (r) =>
      h('div', { style: 'display: flex; gap: 6px' }, [
        h(NButton, { size: 'tiny', secondary: true, onClick: () => downloadFile(r.name) }, () => '下载'),
        h(
          NPopconfirm,
          { onPositiveClick: () => restore(r.name) },
          {
            trigger: () => h(NButton, { size: 'tiny', type: 'warning', secondary: true }, () => '恢复'),
            default: () => `确认将 ${r.name} 标记为待恢复归档？（需重启服务完成恢复）`,
          }
        ),
      ]),
  },
]

function fmtSize(n) {
  if (!n) return '0 B'
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

async function loadConfig() {
  const { data } = await api.get('/config')
  const b = data.config.backup || {}
  cfg.value = {
    enabled: !!b.enabled,
    repo_url: b.repo_url || '',
    pat_env: b.pat_env || 'YUANTUAN_BACKUP_PAT',
    daily_time: b.daily_time || '03:00',
    keep_days: b.keep_days ?? 7,
  }
}

async function loadBackups() {
  try {
    const { data } = await api.get('/backup/list')
    backups.value = data.backups || []
  } catch { /* ignore */ }
}

async function reload() {
  await Promise.all([loadConfig(), loadBackups()])
}

async function runNow() {
  running.value = true
  lastRun.value = ''
  try {
    const { data } = await api.post('/backup/run')
    lastRunOk.value = true
    const pushed = data.pushed ? ' · 已推送远端' : (data.push_error ? ` · 远端未同步` : '')
    lastRun.value = `${data.file} (${fmtSize(data.size_bytes)})${pushed}`
    await loadBackups()
  } catch (e) {
    lastRunOk.value = false
    lastRun.value = e.response?.data?.error || '备份失败'
  } finally {
    running.value = false
  }
}

async function save() {
  saving.value = true
  saveMsg.value = ''
  try {
    const whole = (await api.get('/config')).data.config
    whole.backup = { ...whole.backup, ...cfg.value }
    const { data } = await api.post('/config', { config: whole })
    saveOk.value = true
    saveMsg.value = `已保存（${(data.applied || []).join('、') || '已同步'}）`
  } catch (e) {
    saveOk.value = false
    saveMsg.value = e.response?.data?.error || '保存失败'
  } finally {
    saving.value = false
  }
}

function downloadFile(name) {
  api.get(`/backup/file/${name}`, { responseType: 'blob' }).then((r) => {
    const url = URL.createObjectURL(r.data)
    const a = document.createElement('a')
    a.href = url
    a.download = name
    a.click()
    URL.revokeObjectURL(url)
  })
}

async function restore(name) {
  try {
    const { data } = await api.post('/backup/restore', { file: name })
    lastRunOk.value = true
    lastRun.value = data.message
  } catch (e) {
    lastRunOk.value = false
    lastRun.value = e.response?.data?.error || '恢复标记写入失败'
  }
}

onMounted(reload)
</script>

<style scoped>
.clean-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.form-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 16px;
  padding-top: 14px;
  border-top: 1px solid var(--yt-card-border);
}
.run-msg {
  font-size: 12.5px;
  font-weight: 500;
}
.run-msg.ok { color: var(--yt-ok); }
.run-msg.fail { color: var(--yt-danger); }
</style>
