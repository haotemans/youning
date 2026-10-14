<template>
  <svg :width="size" :height="size" viewBox="0 0 24 24" fill="none"
       stroke="currentColor" :stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <template v-for="(d, i) in paths" :key="i">
      <path :d="d" />
    </template>
  </svg>
</template>

<script setup>
import { computed } from 'vue'

const ICONS = {
  dashboard: ['M3 3h8v9H3z', 'M13 3h8v5h-8z', 'M13 10h8v11h-8z', 'M3 14h8v7H3z'],
  trace: ['M3 12h4l3-7 4 14 3-7h4'],
  tasks: ['M9 6h11', 'M9 12h11', 'M9 18h11', 'M4 6h.01', 'M4 12h.01', 'M4 18h.01'],
  memories: ['M4 5a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z', 'M8 7h8', 'M8 11h8', 'M8 15h5'],
  relations: ['M6 6a3 3 0 1 0 .01 0', 'M18 6a3 3 0 1 0 .01 0', 'M12 18a3 3 0 1 0 .01 0', 'M8.7 7.5l6.6 0', 'M7.5 8.6l3.3 6.8', 'M16.5 8.6l-3.3 6.8'],
  platform: ['M5 12.55a11 11 0 0 1 14.08 0', 'M1.42 9a16 16 0 0 1 21.16 0', 'M8.53 16.11a6 6 0 0 1 6.95 0', 'M12 20h.01'],
  models: ['M12 2L2 7l10 5 10-5-10-5z', 'M2 17l10 5 10-5', 'M2 12l10 5 10-5'],
  personality: ['M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2', 'M12 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8z'],
  meme: ['M4 4h16a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z', 'M8.5 9.5a1.5 1.5 0 1 0 .01 0', 'M21 15l-5-5L5 20'],
  kb: ['M4 19.5A2.5 2.5 0 0 1 6.5 17H20', 'M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z'],
  backup: ['M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4', 'M7 10l5 5 5-5', 'M12 15V3'],
  plugins: ['M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z'],
  mcp: ['M4 6h16v4H4z', 'M4 14h16v4H4z', 'M7 8h.01', 'M7 16h.01'],
  spark: ['M13 2L3 14h9l-1 8 10-12h-9l1-8z'],
  check: ['M20 6L9 17l-5-5'],
  chat: ['M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z'],
  tune: ['M4 21v-7', 'M4 10V3', 'M12 21v-9', 'M12 8V3', 'M20 21v-5', 'M20 12V3', 'M1 14h6', 'M9 8h6', 'M17 16h6'],
}

const props = defineProps({
  name: { type: String, required: true },
  size: { type: [Number, String], default: 18 },
})

const paths = computed(() => ICONS[props.name] || ICONS.spark)
</script>
