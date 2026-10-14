<template>
  <n-config-provider :theme="ui.dark ? darkTheme : null"
                     :theme-overrides="ui.dark ? darkOverrides : lightOverrides">
    <n-message-provider>
      <router-view v-if="$route.name === 'login'" />
      <n-layout v-else style="height: 100vh">
        <n-layout-header class="yt-header">
          <div class="yt-header-left">
            <n-button quaternary size="small" @click="ui.toggleSider" class="yt-icon-btn" aria-label="切换导航栏">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                   stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M4 6h16M4 12h16M4 18h16" />
              </svg>
            </n-button>
            <div class="yt-brand" @click="go('dashboard')" style="cursor: pointer">
              <img :src="mascot" class="yt-brand-logo" alt="Yuantuan" />
              <span class="yt-brand-name">云团</span>
            </div>
          </div>

          <div class="yt-header-right">
            <div class="yt-status-pill" :class="{ online: wsConnected }">
              <span class="yt-status-dot" />
              <span>{{ wsConnected ? '实时在线' : '重连中' }}</span>
            </div>

            <div class="yt-mood-pill" v-if="ui.mood">
              <span class="yt-mood-label">状态</span>
              <span class="yt-mood-val mono">{{ ui.mood }}</span>
            </div>

            <n-button quaternary size="small" @click="ui.toggleTheme" class="yt-theme-btn" aria-label="切换主题">
              <svg v-if="ui.dark" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="12" cy="12" r="5" />
                <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
              </svg>
              <svg v-else width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            </n-button>

            <n-dropdown :options="adminOptions" @select="onAdmin">
              <n-button secondary size="small" class="yt-user-btn">管理员</n-button>
            </n-dropdown>
          </div>
        </n-layout-header>

        <n-layout has-sider position="absolute" style="top: 56px">
          <n-layout-sider class="yt-sider" bordered collapse-mode="width" :width="212" :collapsed-width="64"
                          :collapsed="ui.collapsed" :native-scrollbar="false">
            <n-menu :options="menuOptions" :value="String($route.name)" :collapsed="ui.collapsed"
                    :collapsed-icon-size="18" :root-indent="16" :indent="14" class="yt-nav-menu" @update:value="go" />
          </n-layout-sider>
          <n-layout-content class="yt-main-content" :native-scrollbar="false" content-style="padding: 24px 28px;">
            <router-view v-slot="{ Component }">
              <transition name="fade" mode="out-in">
                <component :is="Component" />
              </transition>
            </router-view>
          </n-layout-content>
        </n-layout>
      </n-layout>
    </n-message-provider>
  </n-config-provider>
</template>

<script setup>
import { h, onMounted, watchEffect } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { darkTheme, NTooltip } from 'naive-ui'
import { useUiStore } from './store/ui'
import { lightOverrides, darkOverrides } from './theme'
import { wsConnected, ensureWs } from './ws'
import { TOKEN_KEY, api } from './api'
import YtIcon from './components/YtIcon.vue'
import mascot from './assets/mascot.png'

const ui = useUiStore()
const route = useRoute()
const router = useRouter()

/** @type {[string, [string, string, string][]][]} */
const groups = [
  ['工作台', [
    ['dashboard', '总览', 'dashboard'],
    ['trace', '决策追踪', 'trace'],
    ['tasks', '任务回放', 'tasks'],
  ]],
  ['数据与认知', [
    ['memories', '记忆库', 'memories'],
    ['relations', '关系图谱', 'relations'],
    ['meme', '表情图库', 'meme'],
    ['kb', '知识库', 'kb'],
  ]],
  ['模型与策略', [
    ['models', '模型服务', 'models'],
    ['personality', '人设版本', 'personality'],
    ['params', '运行参数', 'tune'],
  ]],
  ['接入与扩展', [
    ['platform', '平台接入', 'platform'],
    ['plugins', '插件管理', 'plugins'],
    ['mcp', 'MCP 服务', 'mcp'],
    ['backup', '数据备份', 'backup'],
  ]],
]

const iconWithTip = (icon, label) => () =>
  h(NTooltip, { trigger: 'hover', placement: 'right', disabled: !ui.collapsed }, {
    trigger: () => h(YtIcon, { name: icon, size: 18 }),
    default: () => label,
  })

const menuOptions = groups.map(([label, children]) => ({
  type: 'group',
  label,
  key: label,
  children: children.map(([key, l, icon]) => ({
    label: () => h('span', l),
    key,
    icon: iconWithTip(icon, l),
  })),
}))

function go(key) {
  router.push({ name: key })
}

const adminOptions = [{ label: '退出登录', key: 'logout' }]
function onAdmin(key) {
  if (key === 'logout') {
    localStorage.removeItem(TOKEN_KEY)
    router.push({ name: 'login' })
  }
}

watchEffect(() => {
  document.documentElement.classList.toggle('dark', ui.dark)
})

onMounted(async () => {
  ensureWs()
  try {
    const { data } = await api.get('/dashboard')
    ui.setMood(data.mood)
  } catch { /* ignore */ }
})
</script>
