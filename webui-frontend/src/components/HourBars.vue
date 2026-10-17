<template>
  <div class="hourbars">
    <svg :viewBox="`0 0 ${W} ${H}`" class="hourbars-svg" role="img" aria-label="24 小时消息分布">
      <g v-for="tick in ticks" :key="tick.value">
        <line :x1="PAD" :x2="W - 12" :y1="tick.y" :y2="tick.y" class="hb-grid" />
        <text :x="PAD - 8" :y="tick.y + 4" class="hb-tick" text-anchor="end">{{ tick.value }}</text>
      </g>
      <g v-for="(c, i) in cols" :key="i">
        <rect :x="c.x" :y="BASE - c.inH" :width="BAR_W" :height="Math.max(c.in > 0 ? 2 : 0, c.inH)" class="hb-in" rx="2" />
        <rect :x="c.x + BAR_W + 2" :y="BASE - c.outH" :width="BAR_W" :height="Math.max(c.out > 0 ? 2 : 0, c.outH)" class="hb-out" rx="2" />
        <text v-if="i % 3 === 0 || i === 23" :x="c.x + BAR_W" :y="H - 6" class="hb-tick" text-anchor="middle">{{ String(i).padStart(2, '0') }}:00</text>
        <title>{{ String(i).padStart(2, '0') }}:00 · 收到 {{ c.in }} / 发送 {{ c.out }}</title>
      </g>
    </svg>
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  hours: { type: /** @type {import('vue').PropType<{ in: number, out: number }[]>} */ (Array), default: () => [] },
})

const W = 760, H = 200, PAD = 40, BASE = 172
const SLOT = (W - PAD - 12) / 24, BAR_W = 8.5

const ceiling = computed(() => {
  const max = Math.max(1, ...props.hours.flatMap(h => [h.in || 0, h.out || 0]))
  const step = Math.max(1, Math.pow(10, Math.floor(Math.log10(max))) / 2)
  return Math.ceil(max / (step * 4)) * step * 4
})

const ticks = computed(() => [0, 1, 2, 3, 4].map(i => ({
  value: Math.round(ceiling.value * i / 4),
  y: BASE - i / 4 * 148,
})))

const cols = computed(() => Array.from({ length: 24 }, (_, i) => {
  const h = props.hours[i] || { in: 0, out: 0 }
  return {
    in: h.in || 0,
    out: h.out || 0,
    inH: (h.in || 0) / ceiling.value * 148,
    outH: (h.out || 0) / ceiling.value * 148,
    x: PAD + i * SLOT + (SLOT - BAR_W * 2 - 2) / 2,
  }
}))
</script>

<style scoped>
.hourbars { min-width: 0; }
.hourbars-svg { width: 100%; max-width: 100%; height: auto; min-height: 160px; display: block; }
.hb-grid { stroke: var(--yt-tl-line); stroke-width: 1; stroke-dasharray: 3 3; }
.hb-in { fill: var(--yt-chart-in); }
.hb-out { fill: var(--yt-chart-out); }
.hb-tick { font-size: 11px; fill: var(--yt-ink-3); font-family: var(--yt-mono); }
</style>
