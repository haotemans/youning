<template><span>{{ fmtUptime(elapsed) }}</span></template>

<script setup>
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { fmtUptime } from '../fmt'

const props = defineProps({
  seconds: { type: Number, default: null },
  sampledAt: { type: Number, default: 0 },
  paused: { type: Boolean, default: false },
})
const now = ref(Date.now())
const elapsed = computed(() => props.seconds == null ? null :
  props.seconds + Math.max(0, Math.floor((now.value - props.sampledAt) / 1000)))
watch(() => props.sampledAt, (value) => { now.value = value })
let timer
onMounted(() => {
  timer = setInterval(() => {
    if (!props.paused && !document.hidden) now.value = Date.now()
  }, 1000)
})
onUnmounted(() => clearInterval(timer))
</script>

<style scoped>
span { display: inline-block; min-width: 8ch; font-variant-numeric: tabular-nums; }
</style>
