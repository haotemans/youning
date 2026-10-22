<template>
  <div class="yt-page meme-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">表情图库</span>
        <span class="yt-page-sub mono">待审 {{ counts.pending }} · 已入库 {{ counts.active }}</span>
      </div>
      <n-button size="small" secondary :loading="loading" :disabled="busy" @click="load">刷新</n-button>
    </div>

    <n-card size="small">
      <n-tabs :value="status" type="line" :disabled="busy" @update:value="changeStatus">
        <n-tab name="pending">待审队列 ({{ counts.pending }})</n-tab>
        <n-tab name="active">已入库 ({{ counts.active }})</n-tab>
      </n-tabs>
      <p class="gallery-help">{{ status === 'pending' ? '审核群聊采集的图片，填写分类后入库。不填分类时归入 misc。' : '按分类整理机器人可用的表情包，点击图片查看原图。' }}</p>

      <div class="meme-filter">
        <n-input v-model:value="search" clearable :maxlength="100" :disabled="busy" placeholder="搜索文件名、分类或编号" />
        <n-select v-model:value="category" :options="categoryOptions" clearable filterable :disabled="busy" placeholder="全部分类" />
        <n-select v-model:value="sort" :options="sortOptions" :disabled="busy" aria-label="表情包排序" />
      </div>
      <n-alert v-if="loadError" type="error" class="gallery-message" role="alert">
        {{ loadError }} <n-button size="tiny" :disabled="busy" @click="load">重试加载</n-button>
      </n-alert>
      <div v-if="feedback" role="status" class="gallery-message"><n-alert :type="feedbackError ? 'error' : 'success'">{{ feedback }}</n-alert></div>

      <div class="batch-bar">
        <n-checkbox :checked="allSelected" :indeterminate="selected.length > 0 && !allSelected" :disabled="busy || loading || !items.length" @update:checked="selectPage">选择本页</n-checkbox>
        <span class="selection-count">已选 {{ selected.length }} 张</span>
        <template v-if="selected.length">
          <n-input v-model:value="batchCategory" size="small" :maxlength="40" :disabled="busy" :placeholder="status === 'pending' ? '统一分类（可选）' : '统一分类'" class="batch-category" />
          <n-button size="small" type="primary" secondary :loading="busy" :disabled="loading || (status === 'active' && !batchCategory.trim())" @click="runBatch(status === 'pending' ? 'approve' : 'category')">{{ status === 'pending' ? '批量入库' : '批量改分类' }}</n-button>
          <n-popconfirm positive-text="确认删除" negative-text="取消" :disabled="busy || loading" @positive-click="runBatch('reject')">
            <template #trigger><n-button size="small" type="error" secondary :disabled="busy || loading">批量删除</n-button></template>
            确认永久删除选中的 {{ selected.length }} 张表情包及图片文件？
          </n-popconfirm>
          <n-button size="small" quaternary :disabled="busy" @click="selected = []">取消选择</n-button>
        </template>
      </div>

      <n-spin :show="loading">
        <div v-if="items.length" class="meme-grid">
          <article v-for="m in items" :key="m.id" class="meme-card" :class="{ selected: selected.includes(m.id) }">
            <div class="meme-card-head">
              <n-checkbox :checked="selected.includes(m.id)" :disabled="busy || loading" :aria-label="`选择表情包 ${m.id}`" @update:checked="checked => toggleSelected(m.id, checked)" />
              <span class="mono">#{{ m.id }}</span>
              <n-tag size="tiny" :bordered="false">{{ m.added_by === 'steal' ? '群聊采集' : '手动添加' }}</n-tag>
            </div>
            <div class="meme-preview"><MemeImage :id="m.id" :alt="`表情包 ${m.id}：${m.file}`" /></div>
            <div class="meme-filename" :title="m.file">{{ m.file.split('/').pop() }}</div>
            <div class="meme-meta">
              <n-tag size="tiny" :bordered="false" type="info">{{ m.category === '_inbox' ? '待分类' : m.category || 'misc' }}</n-tag>
              <span class="mono">{{ m.use_count }} 次使用</span>
            </div>
            <n-input v-model:value="drafts[m.id]" size="small" :maxlength="40" :disabled="busy || loading" :placeholder="status === 'pending' ? '入库分类（默认 misc）' : '修改分类'" :input-props="{ 'aria-label': `表情包 ${m.id} 分类` }" />
            <div class="meme-actions">
              <n-button size="small" type="primary" secondary :loading="busyId === m.id" :disabled="busy || loading || (status === 'active' && (!drafts[m.id]?.trim() || drafts[m.id].trim() === m.category))" @click="runSingle(m, status === 'pending' ? 'approve' : 'category')">{{ status === 'pending' ? '入库' : '保存分类' }}</n-button>
              <n-popconfirm positive-text="确认删除" negative-text="取消" :disabled="busy || loading" @positive-click="runSingle(m, 'reject')">
                <template #trigger><n-button size="small" type="error" quaternary :disabled="busy || loading">删除</n-button></template>
                确认永久删除表情包 #{{ m.id }} 及图片文件？
              </n-popconfirm>
            </div>
          </article>
        </div>
        <empty-state v-else-if="!loading && !loadError" :title="search || category ? '没有匹配的表情包' : status === 'pending' ? '暂无待审表情包' : '暂无入库表情包'" />
        <div v-else class="gallery-loading">{{ loading ? '正在加载图库…' : '图库加载失败，请重试' }}</div>
      </n-spin>
      <div class="gallery-footer">
        <span class="mono">共 {{ total }} 张 · 每页 {{ pageSize }} 张</span>
        <n-pagination :page="page" :page-size="pageSize" :item-count="total" :disabled="busy || loading" :page-slot="3" @update:page="changePage" />
      </div>
    </n-card>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { api } from '../api'
import EmptyState from '../components/EmptyState.vue'
import MemeImage from '../components/MemeImage.vue'

type Meme = { id: number; file: string; category: string; use_count: number; added_by: string }
type Action = 'approve' | 'category' | 'reject'
const items = ref<Meme[]>([])
const counts = ref({ pending: 0, active: 0 })
const categories = ref<string[]>([])
const status = ref('pending')
const search = ref('')
const category = ref<string | null>(null)
const sort = ref('newest')
const page = ref(1)
const pageSize = 24
const total = ref(0)
const selected = ref<number[]>([])
const drafts = ref<Record<number, string>>({})
const batchCategory = ref('')
const loading = ref(true)
const busy = ref(false)
const busyId = ref<number | null>(null)
const loadError = ref('')
const feedback = ref('')
const feedbackError = ref(false)
let controller: AbortController | undefined
let searchTimer: ReturnType<typeof setTimeout> | undefined
const categoryOptions = computed(() => categories.value.map(c => ({ label: c === '_inbox' ? '待分类' : c, value: c })))
const sortOptions = [{ label: '最新添加', value: 'newest' }, { label: '最早添加', value: 'oldest' }, { label: '使用最多', value: 'popular' }]
const allSelected = computed(() => items.value.length > 0 && items.value.every(m => selected.value.includes(m.id)))

function errorText(e: any, fallback: string) { return e.response?.data?.error || fallback }
async function load() {
  clearTimeout(searchTimer)
  controller?.abort()
  const request = new AbortController()
  controller = request
  loading.value = true
  loadError.value = ''
  try {
    const { data } = await api.get('/memes', { params: { status: status.value, search: search.value.trim(), category: category.value || undefined, sort: sort.value, page: page.value, page_size: pageSize }, signal: request.signal })
    if (request.signal.aborted) return
    items.value = data.memes
    total.value = data.total
    page.value = data.page
    counts.value = data.counts
    categories.value = data.categories
    selected.value = selected.value.filter(id => items.value.some(m => m.id === id))
    for (const m of items.value) if (!(m.id in drafts.value)) drafts.value[m.id] = status.value === 'pending' ? '' : m.category
  } catch (e) {
    if (!request.signal.aborted) {
      items.value = []
      selected.value = []
      loadError.value = errorText(e, '图库加载失败，请重试')
    }
  } finally {
    if (controller === request) loading.value = false
  }
}
function resetFilter() {
  page.value = 1
  selected.value = []
  load()
}
function changeStatus(value: string) {
  if (busy.value || status.value === value) return
  status.value = value
  items.value = []
  category.value = null
  batchCategory.value = ''
  feedback.value = ''
  resetFilter()
}
function changePage(value: number) { page.value = value; selected.value = []; load() }
function selectPage(checked: boolean) { selected.value = checked ? items.value.map(m => m.id) : [] }
function toggleSelected(id: number, checked: boolean) {
  selected.value = checked ? [...selected.value, id] : selected.value.filter(value => value !== id)
}
watch(search, () => {
  controller?.abort()
  loading.value = true
  clearTimeout(searchTimer)
  searchTimer = setTimeout(resetFilter, 250)
})
watch([category, sort], resetFilter)
async function runSingle(m: Meme, action: Action) {
  if (busy.value) return
  busy.value = true
  busyId.value = m.id
  feedback.value = ''
  try {
    await api.post(`/memes/${m.id}/${action}`, action === 'reject' ? undefined : { category: drafts.value[m.id]?.trim() || undefined })
    delete drafts.value[m.id]
    selected.value = selected.value.filter(id => id !== m.id)
    feedbackError.value = false
    feedback.value = action === 'approve' ? '表情包已入库' : action === 'category' ? '分类已保存' : '表情包已删除'
    await load()
  } catch (e) { feedbackError.value = true; feedback.value = errorText(e, '操作失败，请重试') }
  finally { busy.value = false; busyId.value = null }
}
async function runBatch(action: Action) {
  if (busy.value || !selected.value.length) return
  busy.value = true
  feedback.value = ''
  try {
    const { data } = await api.post('/memes/batch', { ids: selected.value, action, category: batchCategory.value.trim() || undefined })
    for (const id of data.updated) delete drafts.value[id]
    selected.value = data.failed.map((failure: { id: number }) => failure.id)
    feedbackError.value = data.failed.length > 0
    feedback.value = `已处理 ${data.updated.length} 张` + (data.failed.length ? `，${data.failed.length} 张失败：${data.failed.map((f: { id: number; error: string }) => `#${f.id} ${f.error}`).join('；')}` : '')
    await load()
  } catch (e) { feedbackError.value = true; feedback.value = errorText(e, '批量操作失败，请重试') }
  finally { busy.value = false }
}
onMounted(load)
onBeforeUnmount(() => { controller?.abort(); clearTimeout(searchTimer) })
</script>

<style scoped>
.gallery-help { color: var(--yt-ink-3); font-size: 13px; margin: 14px 0; }
.meme-filter { display: grid; grid-template-columns: minmax(180px, 1fr) 180px 150px; gap: 10px; }
.gallery-message { margin-top: 14px; }
.batch-bar { display: flex; flex-wrap: wrap; gap: 10px; align-items: center; min-height: 44px; padding: 14px 0; }
.selection-count { font-size: 12px; color: var(--yt-ink-3); }
.batch-category { width: 180px; }
.meme-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 14px; }
.meme-card { min-width: 0; border: 1px solid var(--yt-card-border); border-radius: 10px; background: var(--yt-elevated); padding: 12px; display: flex; flex-direction: column; gap: 10px; }
.meme-card.selected { border-color: var(--yt-primary); box-shadow: 0 0 0 1px var(--yt-primary); }
.meme-card-head { display: flex; align-items: center; gap: 8px; color: var(--yt-ink-3); font-size: 12px; }
.meme-card-head :deep(.n-tag) { margin-left: auto; }
.meme-preview { background: var(--yt-surface); border: 1px solid var(--yt-card-border); border-radius: 8px; padding: 6px; }
.meme-filename { white-space: nowrap; text-overflow: ellipsis; overflow: hidden; font-size: 12px; color: var(--yt-ink-3); }
.meme-meta { display: flex; justify-content: space-between; align-items: center; gap: 6px; font-size: 12px; color: var(--yt-ink-3); }
.meme-meta :deep(.n-tag) { max-width: 60%; overflow: hidden; }
.meme-actions { display: flex; gap: 8px; align-items: center; }
.meme-actions > :first-child { flex: 1; }
.gallery-loading { min-height: 160px; display: grid; place-items: center; color: var(--yt-ink-3); }
.gallery-footer { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; margin-top: 20px; font-size: 12px; color: var(--yt-ink-3); }
@media (max-width: 600px) {
  .meme-filter { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); }
  .meme-filter > :first-child { grid-column: 1 / -1; }
  .meme-grid { grid-template-columns: minmax(0, 1fr); }
  .batch-category { width: 100%; }
}
</style>
