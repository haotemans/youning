<template>
  <article class="plugin-card" :class="{ 'needs-attention': status.tone === 'warning' || status.tone === 'error' }">
    <button class="plugin-open" @click="$emit('open', plugin, 'overview')" :aria-label="`查看${name}详情`">
      <div class="plugin-heading">
        <span class="plugin-seal" aria-hidden="true">{{ name[0] }}</span>
        <span class="plugin-state" :class="status.tone"><i />{{ status.label }}</span>
      </div>
      <h2>{{ name }}</h2>
      <p class="plugin-purpose">{{ plugin.presentation?.purpose || plugin.description }}</p>
      <p class="plugin-description">{{ plugin.description || '为云团添加新的能力。' }}</p>
      <div class="plugin-call-note">
        <template v-if="plugin.invocations?.active">正在处理请求，完成后自动更新</template>
        <template v-else-if="plugin.invocations?.last_status === 'failed'">最近一次调用未完成，查看原因</template>
        <template v-else-if="plugin.invocations?.last_status === 'succeeded'">最近调用成功 <span>{{ pluginDuration(plugin.invocations.last_duration_ms) }}</span></template>
        <template v-else>等待第一次调用</template>
      </div>
    </button>
    <footer>
      <n-button text class="plugin-configure" @click="$emit('open', plugin, 'config')">配置与用法 <span aria-hidden="true">›</span></n-button>
      <n-switch size="small" :value="plugin.enabled" :loading="busy || plugin.updating" :disabled="busy || plugin.updating || plugin.pending_restart"
        :aria-label="`启用${name}`" @update:value="value => $emit('toggle', plugin, value)" />
    </footer>
  </article>
</template>

<script setup>
import { computed } from 'vue'
import { pluginStatus, pluginDuration } from '../plugin-ui'
const props = defineProps({ plugin: { type: Object, required: true }, busy: Boolean })
defineEmits(['toggle', 'open'])
const name = computed(() => props.plugin.display_name || props.plugin.name)
const status = computed(() => pluginStatus(props.plugin, props.busy))
</script>

<style scoped>
.plugin-card { min-width: 0; background: var(--yt-surface); border: 1px solid var(--yt-card-border); border-radius: 16px; overflow: hidden; }
.plugin-card.needs-attention { border-color: color-mix(in srgb, var(--yt-warning) 45%, var(--yt-card-border)); }
.plugin-open { display: block; width: 100%; padding: 24px 24px 20px; border: 0; text-align: left; color: inherit; background: transparent; cursor: pointer; font: inherit; }
.plugin-open:focus-visible { outline: 2px solid var(--yt-primary); outline-offset: -4px; border-radius: 12px; }
.plugin-open:hover h2 { color: var(--yt-primary); }
.plugin-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 20px; }
.plugin-seal { width: 37px; height: 41px; display: grid; place-items: center; border: 1px solid color-mix(in srgb, var(--yt-primary) 22%, var(--yt-card-border)); border-radius: 9px 9px 12px 9px; background: var(--yt-hover-bg); color: var(--yt-primary); font: 23px 'STSong', 'SimSun', 'Noto Serif SC', serif; }
.plugin-state { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--yt-ink-3); }
.plugin-state i { width: 6px; height: 6px; border-radius: 50%; background: currentColor; }
.plugin-state.success { color: #258073; }
:global(html.dark .plugin-card .plugin-state.success) { color: #74c7b8; }
.plugin-state.working { color: var(--yt-primary); }
.plugin-state.warning { color: var(--yt-jk-num); }
.plugin-state.error { color: var(--yt-danger); }
h2 { margin: 0 0 6px; font: 600 28px/1.35 'STSong', 'SimSun', 'Noto Serif SC', serif; letter-spacing: .1em; color: var(--yt-ink-1); }
.plugin-purpose { margin: 0 0 13px; color: var(--yt-ink-1); font-size: 13px; }
.plugin-description { margin: 0; min-height: 42px; color: var(--yt-ink-3); font-size: 12px; line-height: 1.75; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.plugin-call-note { margin-top: 22px; color: var(--yt-ink-3); font-size: 11px; display: flex; align-items: center; gap: 10px; }
.plugin-call-note span { color: var(--yt-ink-2); font-variant-numeric: tabular-nums; }
footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 14px 24px; border-top: 1px solid var(--yt-card-border); }
.plugin-configure { font-size: 12px; color: var(--yt-ink-2); }
.plugin-configure span { margin-left: 6px; font-size: 19px; line-height: 14px; }
@media (max-width: 520px) { .plugin-open { padding: 20px; } footer { padding: 13px 20px; } }
</style>
