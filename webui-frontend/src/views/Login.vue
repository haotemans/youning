<template>
  <div class="login-split">
    <section class="login-hero-side">
      <div class="login-hero-name">
        <img :src="mascot" class="yt-brand-logo" alt="云团" />
        <span>云团</span>
      </div>
      <p class="login-hero-slogan">
        智能群聊 Agent 管理控制台
      </p>
      <div class="login-hero-points">
        <span class="login-hero-point">
          <yt-icon name="trace" :size="16" />实时决策追踪与任务回放
        </span>
        <span class="login-hero-point">
          <yt-icon name="memories" :size="16" />长期记忆与关系网络图谱
        </span>
        <span class="login-hero-point">
          <yt-icon name="models" :size="16" />多模型路由与扩展插件管理
        </span>
      </div>
    </section>

    <section class="login-form-side">
      <n-card class="login-form-card">
        <div class="login-title">
          {{ needSetup === true ? '初始化管理员密码' : '管理员登录' }}
        </div>
        <n-input v-model:value="pw" type="password" show-password-on="click"
                 :placeholder="needSetup === true ? '设置初始管理员密码' : '输入管理员密码'"
                 size="large" @keyup.enter="doLogin" />
        <n-button type="primary" block size="large" style="margin-top: 16px" :loading="loading" @click="doLogin">
          {{ needSetup === true ? '完成设置并登录' : '登录控制台' }}
        </n-button>
        <n-alert v-if="msg" type="error" :bordered="false" class="yt-login-err">{{ msg }}</n-alert>
      </n-card>
    </section>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import axios from 'axios'
import { TOKEN_KEY } from '../api'
import { reconnectWs, ensureWs } from '../ws'
import YtIcon from '../components/YtIcon.vue'
import mascot from '../assets/mascot.png'

const pw = ref('')
const msg = ref('')
const loading = ref(false)
const router = useRouter()
const needSetup = ref(null)

onMounted(async () => {
  try {
    const { data } = await axios.get('/api/auth/status')
    needSetup.value = !!data.need_setup
  } catch { /* ignore */ }
})

async function doLogin() {
  msg.value = ''
  if (!pw.value) { msg.value = '请输入密码'; return }
  loading.value = true
  try {
    const { data } = await axios.post('/api/auth/login', { password: pw.value })
    localStorage.setItem(TOKEN_KEY, data.token)
    reconnectWs(); ensureWs()
    router.push({ name: 'dashboard' })
  } catch (e) {
    msg.value = e.response?.data?.error || '登录失败'
  } finally {
    loading.value = false
  }
}
</script>

<style scoped>
.login-title {
  font-size: 18px;
  font-weight: 600;
  letter-spacing: -0.01em;
  margin-bottom: 18px;
  color: var(--yt-ink-1);
}
.yt-login-err {
  margin-top: 14px;
  border-radius: 8px;
}
.login-form-card :deep(.n-card__content) {
  padding: 28px 28px 30px;
}
</style>
