<template>
  <div class="cap-card" :class="{ 'is-disabled': !plugin.enabled, 'is-dirty': dirty }" @click="$emit('open', plugin)">
    <div class="cap-card-head">
      <div class="cap-card-icon plugin">
        {{ (plugin.display_name || plugin.name)[0].toUpperCase() }}
      </div>
      <div class="cap-card-title">
        <div class="cap-card-name">{{ plugin.display_name || plugin.name }}</div>
        <div class="cap-card-status">
          <span v-if="plugin.version" class="mono">v{{ plugin.version }} · </span>
          <span v-if="plugin.enabled && plugin.loaded" class="ok">已加载</span>
          <span v-else-if="plugin.enabled && !plugin.loaded" class="warn">待重启</span>
          <span v-else>已禁用</span>
        </div>
      </div>
    </div>

    <div class="cap-card-body">
      <div class="cap-card-desc">
        {{ plugin.description || '内置扩展插件' }}
      </div>
      <div class="cap-card-tags">
        <span v-if="(plugin.skills || []).length" class="chip skill">
          Skills {{ plugin.skills.length }}
        </span>
        <span v-if="(plugin.tools || []).length" class="chip tool">
          Tools {{ plugin.tools.length }}
        </span>
        <span v-if="plugin.data_dir" class="chip">Data</span>
      </div>
    </div>

    <div class="cap-card-foot" @click.stop>
      <n-switch
        size="small"
        :value="plugin.enabled"
        @update:value="(v) => $emit('toggle', plugin, v)"
      />
      <span class="spacer" />
      <span class="cap-card-open" @click="$emit('open', plugin)">详情</span>
    </div>
  </div>
</template>

<script setup>
defineProps({
  plugin: { type: Object, required: true },
  dirty: { type: Boolean, default: false },
})
defineEmits(['toggle', 'open'])
</script>

<style scoped>
.ok { color: var(--yt-ok); font-weight: 600; }
.warn { color: var(--yt-warning); font-weight: 600; }
</style>
