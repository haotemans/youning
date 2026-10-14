<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">平台接入</span>
        <span class="yt-page-sub mono">OneBot 11</span>
      </div>
      <n-space align="center" :size="10">
        <n-tag size="small" :type="d.adapter_connected ? 'success' : 'default'" :bordered="false">
          {{ d.adapter_connected ? '已连接' : '离线' }}
        </n-tag>
        <n-button size="small" secondary @click="reload">刷新</n-button>
      </n-space>
    </div>

    <n-grid cols="1 s:2 l:4" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" :style="{ background: d.adapter_connected ? 'var(--yt-ok)' : 'var(--yt-danger)' }" />
            通道状态
          </div>
          <div class="stat-foot">
            <span class="stat-num" :style="{ color: d.adapter_connected ? 'var(--yt-ok)' : 'var(--yt-ink-1)' }">
              {{ d.adapter_connected ? '在线' : '未连接' }}
            </span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-primary)" />
            当前账号
          </div>
          <div class="stat-foot">
            <span class="stat-num mono">{{ d.self_qq || '—' }}</span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-violet)" />
            反向 WebSocket 端点
          </div>
          <div class="stat-foot">
            <span class="stat-num mono" style="font-size: 16px">ws://{{ form.listen_addr || '127.0.0.1:6199' }}/ws</span>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card class="stat-card" size="small">
          <div class="stat-label">
            <span class="stat-chip" style="background: var(--yt-warning)" />
            鉴权 Token
          </div>
          <div class="stat-foot">
            <span class="stat-num">{{ hasToken ? '已设置' : '未设置' }}</span>
          </div>
        </n-card>
      </n-gi>
    </n-grid>

    <n-card title="NapCat 反向 WebSocket 配置" size="small">
      <n-form label-placement="left" label-width="110" :show-feedback="false" class="clean-form" style="max-width: 560px">
        <n-form-item label="启用接入">
          <n-switch v-model:value="form.enabled" />
        </n-form-item>
        <n-form-item label="监听地址">
          <n-input v-model:value="form.listen_addr" placeholder="127.0.0.1:6199" />
        </n-form-item>
        <n-form-item label="连接 Token">
          <n-input v-model:value="form.token" type="password" show-password-on="click"
                   placeholder="留空则保持原 Token 不变" />
        </n-form-item>
      </n-form>

      <div class="form-actions">
        <n-button type="primary" size="small" :loading="saving" @click="save(false)">保存配置</n-button>
        <n-button type="warning" size="small" secondary :loading="saving || restarting" @click="save(true)">保存并重启服务</n-button>
        <span v-if="msg" :class="['save-msg', ok ? 'ok' : 'fail']">{{ msg }}</span>
      </div>
    </n-card>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import { api } from '../api'
import { subscribeWs } from '../ws'

/** @type {import('vue').Ref<Record<string, any>>} */
const d = ref({ adapter_connected: false, self_qq: 0 })
const form = ref({ enabled: true, listen_addr: '127.0.0.1:6199', token: '' })
const hasToken = ref(false)
/** @type {import('vue').Ref<Record<string, any>>} */
const whole = ref({})
const saving = ref(false)
const restarting = ref(false)
const msg = ref('')
const ok = ref(false)

async function loadDashboard() {
  try {
    const { data } = await api.get('/dashboard')
    d.value.adapter_connected = !!data.adapter_connected
    d.value.self_qq = data.self_qq || 0
  } catch { /* ignore */ }
}

async function loadConfig() {
  const { data } = await api.get('/config')
  whole.value = data.config
  const n = data.config.napcat || {}
  form.value = {
    enabled: n.enabled ?? true,
    listen_addr: n.listen_addr ?? '127.0.0.1:6199',
    token: '',
  }
  hasToken.value = !!(n.token && n.token !== '')
}

async function reload() {
  await Promise.all([loadDashboard(), loadConfig()])
}

async function save(andRestart = false) {
  saving.value = true
  msg.value = ''
  try {
    const newNapcat = { ...whole.value.napcat }
    newNapcat.enabled = form.value.enabled
    newNapcat.listen_addr = form.value.listen_addr
    delete newNapcat.ws_url
    if (form.value.token) newNapcat.token = form.value.token
    const config = { ...whole.value, napcat: newNapcat }
    const { data } = await api.post('/config', { config })
    whole.value = config
    form.value.token = ''
    ok.value = true
    if (andRestart) {
      restarting.value = true
      msg.value = '已保存，正在重启服务…'
      const reloadTimer = setTimeout(() => { window.location.reload() }, 3200)
      try {
        await api.post('/config/restart')
      } catch (err) {
        if (err.response?.status === 409) {
          clearTimeout(reloadTimer)
          restarting.value = false
          ok.value = false
          msg.value = err.response?.data?.error || '容器环境请使用 docker compose restart'
          saving.value = false
          return
        }
      }
      return
    }
    const restarts = (data.requires_restart || []).filter(r => r.includes('napcat'))
    const parts = []
    if ((data.applied || []).some(a => a.includes('token'))) parts.push('Token 已生效')
    if (restarts.length) parts.push('监听地址变更需重启服务')
    msg.value = `已保存${parts.length ? '（' + parts.join('，') + '）' : ''}`
    await loadConfig()
  } catch (e) {
    ok.value = false
    msg.value = e.response?.data?.error || '保存失败'
  } finally {
    saving.value = false
  }
}

function onWsEvent(ev) {
  if (!ev || !ev.kind) return
  if (ev.kind === 'ConfigReloaded') loadConfig()
}

let timer = null
let off = null
onMounted(() => {
  reload()
  timer = setInterval(loadDashboard, 8000)
  off = subscribeWs(onWsEvent)
})
onUnmounted(() => { clearInterval(timer); off && off() })
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
  margin-top: 18px;
  padding-top: 14px;
  border-top: 1px solid var(--yt-card-border);
}
.save-msg {
  font-size: 12.5px;
  font-weight: 500;
}
.save-msg.ok { color: var(--yt-ok); }
.save-msg.fail { color: var(--yt-danger); }
</style>
