<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">人设版本</span>
        <span class="yt-page-sub mono">{{ versions.length }} 个版本</span>
      </div>
      <n-button size="small" secondary @click="load">刷新</n-button>
    </div>

    <n-grid cols="1 l:2" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card title="提示词编辑器" size="small">
          <n-input v-model:value="draft" type="textarea" :rows="16" placeholder="输入人设系统提示词" />
          <div class="editor-bar">
            <n-input v-model:value="note" size="small" placeholder="版本变更备注" style="flex: 1" />
            <n-button type="primary" size="small" :loading="saving" @click="saveNew">保存并激活新版本</n-button>
          </div>
          <div v-if="msg" :class="['status-text', ok ? 'ok' : 'fail']">{{ msg }}</div>
        </n-card>

        <n-card v-if="diffLines" title="版本差异对比" size="small" style="margin-top: 16px">
          <div class="diff-box mono">
            <div v-for="(l, i) in diffLines" :key="i" class="diff-line" :style="{ background: l.bg }">
              <span class="diff-tag">{{ l.tag }}</span>
              <span>{{ l.text }}</span>
            </div>
          </div>
        </n-card>
      </n-gi>

      <n-gi>
        <n-card title="版本历史" size="small">
          <div v-if="versions.length" class="version-list">
            <div v-for="v in versions" :key="v.version_no" class="version-row" :class="{ active: v.active }">
              <div class="version-main">
                <div class="version-head">
                  <span class="version-no mono">v{{ v.version_no }}</span>
                  <n-tag v-if="v.active" size="tiny" type="success" :bordered="false">当前生效</n-tag>
                  <span class="version-note">{{ v.note || '无备注' }}</span>
                </div>
                <div class="version-meta mono">
                  {{ fmtTs(v.created_at) }} · {{ v.content_len }} 字符
                </div>
              </div>
              <div class="version-actions">
                <n-checkbox :checked="diffSel.includes(v.version_no)" @update:checked="(c) => toggleDiff(v.version_no, c)">
                  对比
                </n-checkbox>
                <n-button size="tiny" secondary @click="loadOne(v.version_no)">载入</n-button>
                <n-popconfirm @positive-click="rollback(v.version_no)">
                  <template #trigger>
                    <n-button size="tiny" type="warning" secondary>回滚</n-button>
                  </template>
                  确认以 v{{ v.version_no }} 的内容生成并激活新版本？
                </n-popconfirm>
              </div>
            </div>
          </div>
          <empty-state v-else title="暂无历史版本" />
        </n-card>
      </n-gi>
    </n-grid>
  </div>
</template>

<script setup>
import { onMounted, ref } from 'vue'
import { api } from '../api'
import { fmtTs } from '../fmt'
import EmptyState from '../components/EmptyState.vue'

const versions = ref([])
const draft = ref('')
const note = ref('')
const saving = ref(false)
const msg = ref('')
const ok = ref(false)
const diffSel = ref([])
const diffLines = ref(null)
const contents = ref({})

async function load() {
  const { data } = await api.get('/personality/versions')
  versions.value = data.versions || []
  const active = versions.value.find((v) => v.active)
  if (active && !draft.value) loadOne(active.version_no)
}
async function loadOne(no) {
  const { data } = await api.get(`/personality/versions/${no}`)
  draft.value = data.content
  contents.value[no] = data.content
}
async function saveNew() {
  if (!draft.value.trim()) { ok.value = false; msg.value = '提示词内容不能为空'; return }
  saving.value = true
  try {
    const { data } = await api.post('/personality/versions', { content: draft.value, note: note.value })
    ok.value = true
    msg.value = `已保存 v${data.version_no} 并设为当前生效版本`
    note.value = ''
    await load()
  } catch (e) {
    ok.value = false
    msg.value = e.response?.data?.error || '保存失败'
  } finally {
    saving.value = false
  }
}
async function rollback(no) {
  try {
    const { data } = await api.post(`/personality/rollback/${no}`)
    ok.value = true
    msg.value = `已回滚并生成 v${data.version_no}`
    await load()
  } catch (e) {
    ok.value = false
    msg.value = e.response?.data?.error || '回滚失败'
  }
}
async function toggleDiff(no, checked) {
  const s = diffSel.value.filter((x) => x !== no)
  if (checked) s.push(no)
  diffSel.value = s.slice(-2)
  if (diffSel.value.length === 2) {
    const [a, b] = [...diffSel.value].sort((x, y) => x - y)
    for (const n of [a, b]) if (!contents.value[n]) {
      const { data } = await api.get(`/personality/versions/${n}`)
      contents.value[n] = data.content
    }
    diffLines.value = lineDiff(contents.value[a], contents.value[b])
  } else {
    diffLines.value = null
  }
}

function lineDiff(a, b) {
  const A = a.split('\n'); const B = b.split('\n')
  const n = Math.max(A.length, B.length)
  const out = []
  for (let i = 0; i < n; i++) {
    if (A[i] === B[i]) out.push({ tag: ' ', text: A[i] ?? '', bg: 'transparent' })
    else if (A[i] !== undefined && B[i] !== undefined) out.push({ tag: '~', text: `- ${A[i]}  →  + ${B[i]}`, bg: 'var(--yt-warning-soft)' })
    else if (A[i] !== undefined) out.push({ tag: '-', text: A[i], bg: 'var(--yt-danger-soft)' })
    else out.push({ tag: '+', text: B[i], bg: 'var(--yt-ok-soft)' })
  }
  return out
}

onMounted(load)
</script>

<style scoped>
.editor-bar {
  display: flex;
  gap: 10px;
  align-items: center;
  margin-top: 12px;
}
.status-text {
  margin-top: 8px;
  font-size: 12.5px;
  font-weight: 500;
}
.status-text.ok { color: var(--yt-ok); }
.status-text.fail { color: var(--yt-danger); }

.version-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.version-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  background: var(--yt-elevated);
  flex-wrap: wrap;
}
.version-row.active {
  background: var(--yt-surface);
  border-color: var(--yt-primary);
}
.version-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.version-no {
  font-weight: 600;
  font-size: 13.5px;
  color: var(--yt-ink-1);
}
.version-note {
  font-size: 13px;
  color: var(--yt-ink-2);
}
.version-meta {
  font-size: 11.5px;
  color: var(--yt-ink-3);
  margin-top: 4px;
}
.version-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
.diff-box {
  font-size: 12px;
  max-height: 320px;
  overflow: auto;
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  background: var(--yt-code-bg);
  padding: 8px 10px;
}
.diff-line {
  white-space: pre-wrap;
  padding: 2px 6px;
  border-radius: 4px;
}
.diff-tag {
  display: inline-block;
  width: 16px;
  color: var(--yt-ink-3);
}
</style>
