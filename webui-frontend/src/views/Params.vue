<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">运行参数</span>
      </div>
      <n-space align="center" :size="10">
        <span v-if="text" :class="['save-msg', ok ? 'ok' : 'fail']">{{ text }}</span>
        <n-button type="primary" size="small" :loading="saving" @click="save">保存运行参数</n-button>
      </n-space>
    </div>

    <n-alert v-if="restartList.length" type="warning" :bordered="false">
      以下配置项需重启服务后生效：{{ restartList.join('、') }}
    </n-alert>

    <n-skeleton v-if="!loaded" text :repeat="10" />
    <template v-else>
      <n-grid cols="1 m:2" :x-gap="16" :y-gap="16" responsive="screen">
        <n-gi>
          <div class="param-col">
            <n-card title="夜间记忆归纳" size="small">
              <n-form label-placement="left" label-width="150" :show-feedback="false" class="clean-form">
                <n-form-item label="启用定时归纳">
                  <n-switch v-model:value="f.consolidation.enabled" />
                </n-form-item>
                <n-form-item label="每日执行时刻">
                  <n-time-picker :formatted-value="f.consolidation.daily_time" format="HH:mm"
                                 value-format="HH:mm" style="width: 160px"
                                 @update:formatted-value="(v) => (f.consolidation.daily_time = v || '03:00')" />
                </n-form-item>
                <n-form-item label="启动时立即执行">
                  <n-switch v-model:value="f.consolidation.run_on_startup" />
                </n-form-item>
              </n-form>
            </n-card>

            <n-card title="上下文预算" size="small">
              <n-form label-placement="left" label-width="150" :show-feedback="false" class="clean-form">
                <n-form-item label="字符预算上限">
                  <n-input-number v-model:value="f.context.budget_chars" :min="4000" :max="200000" :step="1000" style="width: 160px" />
                </n-form-item>
                <n-form-item label="近期会话窗口 (条)">
                  <n-input-number v-model:value="f.context.k" :min="5" :max="100" style="width: 160px" />
                </n-form-item>
                <n-form-item label="每人注入记忆 (条)">
                  <n-input-number v-model:value="f.context.roster_mem_per" :min="0" :max="10" style="width: 160px" />
                </n-form-item>
              </n-form>
            </n-card>

            <n-card title="频控与节流" size="small">
              <n-form label-placement="left" label-width="150" :show-feedback="false" class="clean-form">
                <n-form-item label="统计窗口 (秒)">
                  <n-input-number v-model:value="f.prefilter.window_secs" :min="5" :max="600" style="width: 160px" />
                </n-form-item>
                <n-form-item label="窗口回复上限">
                  <n-input-number v-model:value="f.prefilter.self_msg_cap" :min="1" :max="100" style="width: 160px" />
                </n-form-item>
                <n-form-item label="决策调用上限 (次/分)">
                  <n-input-number v-model:value="f.prefilter.decision_cost_per_min" :min="1" :max="120" style="width: 160px" />
                </n-form-item>
              </n-form>
            </n-card>
          </div>
        </n-gi>

        <n-gi>
          <div class="param-col">
            <n-card title="回复形态与延时" size="small">
              <n-form label-placement="left" label-width="150" :show-feedback="false" class="clean-form">
                <n-form-item label="单次回复气泡上限">
                  <n-input-number v-model:value="f.reply.bubble_cap" :min="1" :max="5" style="width: 160px" />
                </n-form-item>
                <n-form-item label="单泡目标字数">
                  <n-input-number v-model:value="f.reply.bubble_char_cap" :min="20" :max="2000" :step="10" style="width: 160px" />
                </n-form-item>
                <n-form-item label="单字延时系数 (ms)">
                  <n-input-number v-model:value="f.reply.per_char_ms" :min="0" :max="500" style="width: 160px" />
                </n-form-item>
                <n-form-item label="首泡最小延时 (ms)">
                  <n-input-number v-model:value="f.reply.first_delay_min_ms" :min="0" :max="10000" :step="50" style="width: 160px" />
                </n-form-item>
                <n-form-item label="首泡最大延时 (ms)">
                  <n-input-number v-model:value="f.reply.first_delay_max_ms" :min="0" :max="30000" :step="50" style="width: 160px" />
                </n-form-item>
                <n-form-item label="单泡延时上限 (ms)">
                  <n-input-number v-model:value="f.reply.max_delay_ms" :min="100" :max="30000" :step="100" style="width: 160px" />
                </n-form-item>
                <n-form-item label="总延时预算 (秒)">
                  <n-input-number v-model:value="totalBudgetSecs" :min="1" :max="60" style="width: 160px" />
                </n-form-item>
              </n-form>
            </n-card>

            <n-card title="消息管线与表情采集" size="small">
              <n-form label-placement="left" label-width="150" :show-feedback="false" class="clean-form">
                <n-form-item label="单会话队列容量">
                  <n-input-number v-model:value="f.pipeline.per_chat_queue_cap" :min="4" :max="512" style="width: 160px" />
                </n-form-item>
                <n-form-item label="自发消息缓存数">
                  <n-input-number v-model:value="f.pipeline.self_msg_ids_cap" :min="32" :max="4096" style="width: 160px" />
                </n-form-item>
                <n-form-item label="初始决策速率上限">
                  <n-input-number v-model:value="f.pipeline.decision_cost_per_min_init" :min="1" :max="120" style="width: 160px" />
                </n-form-item>
                <n-form-item label="自动采集群表情">
                  <n-switch v-model:value="f.meme.steal_enabled" />
                </n-form-item>
              </n-form>
            </n-card>
          </div>
        </n-gi>
      </n-grid>
    </template>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { api } from '../api'

const loaded = ref(false)
const saving = ref(false)
const text = ref('')
const ok = ref(false)
const restartList = ref([])

const f = ref({
  consolidation: { enabled: true, daily_time: '03:00', run_on_startup: false },
  context: { budget_chars: 40000, k: 20, roster_mem_per: 3 },
  reply: {
    first_delay_min_ms: 300, first_delay_max_ms: 800, base_delay_ms: 600,
    per_char_ms: 40, jitter_ratio: 0.3, min_delay_ms: 800, max_delay_ms: 4000,
    total_budget_ms: 8000, bubble_cap: 3, bubble_char_cap: 500,
  },
  prefilter: { window_secs: 60, self_msg_cap: 12, decision_cost_per_min: 30 },
  pipeline: {
    per_chat_queue_cap: 32,
    self_msg_ids_cap: 512,
    decision_cost_per_min_init: 30,
  },
  meme: { steal_enabled: true },
})
const whole = ref({})

const totalBudgetSecs = computed({
  get: () => Math.round(f.value.reply.total_budget_ms / 1000),
  set: (v) => { f.value.reply.total_budget_ms = Math.max(1, v || 1) * 1000 },
})

onMounted(async () => {
  const { data } = await api.get('/config')
  const c = data.config || {}
  whole.value = c
  for (const k of ['consolidation', 'context', 'reply', 'prefilter', 'pipeline', 'meme']) {
    if (c[k]) f.value[k] = { ...f.value[k], ...c[k] }
  }
  loaded.value = true
})

async function save() {
  saving.value = true
  text.value = ''
  restartList.value = []
  try {
    const config = {
      ...whole.value,
      consolidation: { ...f.value.consolidation },
      context: { ...f.value.context },
      reply: { ...f.value.reply },
      prefilter: { ...f.value.prefilter },
      pipeline: { ...f.value.pipeline },
      meme: { ...f.value.meme },
    }
    const { data } = await api.post('/config', { config })
    ok.value = true
    whole.value = config
    text.value = `已保存（${(data.applied || []).join('、') || '已同步'}）`
    restartList.value = data.requires_restart || []
  } catch (e) {
    ok.value = false
    text.value = e.response?.data?.error || '保存失败'
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.param-col {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.clean-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.save-msg {
  font-size: 12.5px;
  font-weight: 500;
}
.save-msg.ok { color: var(--yt-ok); }
.save-msg.fail { color: var(--yt-danger); }
</style>
