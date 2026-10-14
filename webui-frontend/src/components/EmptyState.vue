<template>
  <div class="empty-state">
    <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor"
         stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="3" y="4" width="18" height="16" rx="3" />
      <path d="M3 10h18" />
      <path d="M8 15h8" />
    </svg>
    <div class="empty-state-title">{{ title }}</div>
  </div>
</template>

<script setup>
defineProps({
  title: { type: String, default: '暂无数据' },
  hint: { type: String, default: '' },
})
</script>
