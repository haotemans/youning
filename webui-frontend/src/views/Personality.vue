<template>
  <div class="yt-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">人设设置</span>
        <span class="yt-page-sub mono">{{ versions.length }} 个版本</span>
      </div>
      <n-button size="small" secondary :disabled="saving || loading" @click="load()">刷新</n-button>
    </div>

    <n-grid cols="1 l:2" :x-gap="16" :y-gap="16" responsive="screen">
      <n-gi>
        <n-card title="人物性格系统提示词" size="small">
          <template #header-extra>
            <n-button size="tiny" secondary :disabled="saving || loading" @click="loadDefaults">载入云团模板</n-button>
          </template>
          <p class="editor-help">定义云团的性格、回答长度和表达方式。模板包含专业简洁的技术回答与自然俏皮的日常聊天风格。</p>
          <n-input v-model:value="draft" type="textarea" :rows="16" :maxlength="16000" :disabled="saving || loading" placeholder="输入人物性格系统提示词" />
          <div class="examples-head">
            <span>回复对话示例 · {{ examples.length }} / 20</span>
            <n-button size="tiny" secondary :disabled="saving || loading || examples.length >= 20" @click="examples.push({ question: '', answer: '' })">添加一问一答</n-button>
          </div>
          <p class="editor-help">通过一问一答让云团参考语气和篇幅。示例仅用于学习表达风格，保存后与系统提示词一起生效。</p>
          <div v-for="(example, i) in examples" :key="i" class="example-pair">
            <div class="example-head">
              <span>示例 {{ i + 1 }}</span>
              <n-button size="tiny" quaternary type="error" :disabled="saving || loading" @click="examples.splice(i, 1)">删除示例 {{ i + 1 }}</n-button>
            </div>
            <label :for="`example-question-${i}`">用户提问</label>
            <n-input v-model:value="example.question" :input-props="{ id: `example-question-${i}` }" type="textarea" :autosize="{ minRows: 2, maxRows: 5 }" :maxlength="1000" :disabled="saving || loading" placeholder="例如：Python 的列表和元组有什么区别？" />
            <label :for="`example-answer-${i}`">云团回复</label>
            <n-input v-model:value="example.answer" :input-props="{ id: `example-answer-${i}` }" type="textarea" :autosize="{ minRows: 3, maxRows: 10 }" :maxlength="4000" :disabled="saving || loading" placeholder="填写希望云团参考的回答，可包含代码块" />
          </div>
          <p v-if="!examples.length" class="editor-help">还没有回复示例，可以添加一组，或载入云团模板。</p>
          <div class="editor-bar">
            <n-input v-model:value="note" size="small" :disabled="saving || loading" placeholder="版本变更备注" style="flex: 1" />
            <n-button type="primary" size="small" :loading="saving" :disabled="loading" @click="saveNew">保存并激活新版本</n-button>
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
                  {{ fmtTs(v.created_at) }} · {{ v.content_len }} 字符 · {{ v.reply_examples_count || 0 }} 组示例
                </div>
              </div>
              <div class="version-actions">
                <n-checkbox :checked="diffSel.includes(v.version_no)" @update:checked="(c) => toggleDiff(v.version_no, c)">
                  对比
                </n-checkbox>
                <n-button size="tiny" secondary :disabled="saving || loading" @click="loadOne(v.version_no)">载入</n-button>
                <n-popconfirm positive-text="确认" negative-text="取消" :disabled="saving || loading" @positive-click="rollback(v.version_no)">
                  <template #trigger>
                    <n-button size="tiny" type="warning" secondary :disabled="saving || loading">回滚</n-button>
                  </template>
                  确认以 v{{ v.version_no }} 的提示词和回复示例生成并激活新版本？
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
const examples = ref([])
const loading = ref(true)
const loaded = ref(false)
const note = ref('')
const saving = ref(false)
const msg = ref('')
const ok = ref(false)
const diffSel = ref([])
const diffLines = ref(null)
const contents = ref({})

function reportError(e, fallback) {
  ok.value = false
  msg.value = e.response?.data?.error || fallback
}
function applyDraft(data) {
  draft.value = data.content || ''
  examples.value = (data.reply_examples || []).map(e => ({ question: e.question, answer: e.answer }))
  loaded.value = true
}
function diffText(data) {
  return `${data.content}\n\n回复对话示例：\n${JSON.stringify(data.reply_examples || [], null, 2)}`
}
async function load(reloadDraft = false) {
  loading.value = true
  try {
    const { data } = await api.get('/personality/versions')
    versions.value = data.versions || []
    if (reloadDraft || !loaded.value) {
      const active = versions.value.find((v) => v.active)
      if (active) await loadOne(active.version_no)
      else await loadDefaults()
    }
  } catch (e) { reportError(e, '加载人设版本失败') }
  finally { loading.value = false }
}
async function loadOne(no) {
  loading.value = true
  try {
    const { data } = await api.get(`/personality/versions/${no}`)
    applyDraft(data)
    contents.value[no] = diffText(data)
  } catch (e) { reportError(e, '载入人设版本失败') }
  finally { loading.value = false }
}
async function loadDefaults() {
  loading.value = true
  try {
    const { data } = await api.get('/personality/defaults')
    applyDraft(data)
    note.value = '云团性格与回复示例'
    ok.value = true
    msg.value = '已载入云团模板，保存并激活后生效'
  } catch (e) { reportError(e, '载入云团模板失败') }
  finally { loading.value = false }
}
async function saveNew() {
  if (!draft.value.trim()) { ok.value = false; msg.value = '提示词内容不能为空'; return }
  const incomplete = examples.value.findIndex(e => !e.question.trim() || !e.answer.trim())
  if (incomplete >= 0) { ok.value = false; msg.value = `第 ${incomplete + 1} 组示例的问题和回答都不能为空`; return }
  saving.value = true
  try {
    const { data } = await api.post('/personality/versions', { content: draft.value, reply_examples: examples.value, note: note.value })
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
  saving.value = true
  try {
    const { data } = await api.post(`/personality/rollback/${no}`)
    ok.value = true
    msg.value = `已回滚并生成 v${data.version_no}`
    await load(true)
  } catch (e) {
    ok.value = false
    msg.value = e.response?.data?.error || '回滚失败'
  } finally { saving.value = false }
}
async function toggleDiff(no, checked) {
  const s = diffSel.value.filter((x) => x !== no)
  if (checked) s.push(no)
  diffSel.value = s.slice(-2)
  if (diffSel.value.length === 2) {
    const [a, b] = [...diffSel.value].sort((x, y) => x - y)
    try {
      for (const n of [a, b]) if (!contents.value[n]) {
        const { data } = await api.get(`/personality/versions/${n}`)
        contents.value[n] = diffText(data)
      }
      if (diffSel.value.includes(a) && diffSel.value.includes(b)) {
        diffLines.value = lineDiff(contents.value[a], contents.value[b])
      }
    } catch (e) {
      diffLines.value = null
      reportError(e, '加载版本对比失败')
    }
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
.editor-help {
  color: var(--yt-ink-3);
  font-size: 12.5px;
  line-height: 1.7;
  margin: 0 0 12px;
}
.examples-head, .example-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin: 16px 0 8px;
}
.example-pair {
  border: 1px solid var(--yt-card-border);
  border-radius: 8px;
  background: var(--yt-elevated);
  padding: 12px;
  margin-bottom: 12px;
}
.example-head { margin-top: 0; }
.example-pair label {
  display: block;
  font-size: 12px;
  color: var(--yt-ink-2);
  margin: 8px 0 4px;
}
.editor-bar {
  display: flex;
  gap: 10px;
  align-items: center;
  margin-top: 12px;
  flex-wrap: wrap;
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
