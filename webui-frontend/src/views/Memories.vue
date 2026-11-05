<template>
  <div class="yt-page memories-page">
    <div class="yt-page-head">
      <div class="yt-page-head-left">
        <span class="yt-page-title">记忆库</span>
        <span class="yt-page-sub mono">{{ peopleTotal }} 位人物</span>
      </div>
      <n-button size="small" secondary :loading="loading || peopleLoading" @click="refresh">刷新</n-button>
    </div>

    <div class="yt-toolbar memory-toolbar" :class="{ 'profile-toolbar': tab === 'profile' }">
      <n-input v-model:value="search" clearable :maxlength="100" placeholder="搜索昵称、QQ 或记忆内容" aria-label="搜索记忆" />
      <n-select v-if="tab !== 'profile'" v-model:value="conversationKey" :options="conversationOptions" :loading="conversationsLoading" :disabled="conversationsLoading" clearable filterable placeholder="全部会话" aria-label="来源会话" />
    </div>

    <div class="memory-layout">
      <n-card size="small" class="people-panel">
        <div class="people-heading">人物</div>
        <button type="button" class="person-option" :class="{ active: !selectedPerson }" @click="choosePerson(null)">
          <span class="memory-avatar"><YtIcon name="personality" /></span><span>全部画像</span>
        </button>
        <n-spin :show="peopleLoading">
          <n-alert v-if="peopleError" type="error" class="memory-alert" role="alert">{{ peopleError }} <n-button text size="tiny" @click="loadPeople">重试</n-button></n-alert>
          <button v-for="person in people" :key="person.id" type="button" class="person-option" :class="{ active: selectedPerson?.id === person.id }" :aria-pressed="selectedPerson?.id === person.id" @click="choosePerson(person)">
            <span class="memory-avatar"><img v-if="avatar(person)" :src="avatar(person)" alt="" loading="lazy" referrerpolicy="no-referrer" @error="badAvatars.add(person.id)" /><span v-else>{{ initial(person) }}</span></span>
            <span class="identity-text"><span class="identity-name">{{ person.name }}</span><span class="identity-id mono">{{ person.qq || person.id }}</span></span>
          </button>
          <div v-if="!peopleLoading && !peopleError && !people.length" class="memory-empty-small">暂无匹配人物</div>
        </n-spin>
        <div class="people-pagination">
          <n-button size="tiny" quaternary :disabled="peoplePage === 1 || peopleLoading" @click="changePeoplePage(-1)">上一页</n-button>
          <span class="mono">{{ peoplePage }}</span>
          <n-button size="tiny" quaternary :disabled="!peopleHasMore || peopleLoading" @click="changePeoplePage(1)">下一页</n-button>
        </div>
      </n-card>

      <n-card size="small" class="memory-content">
        <div v-if="selectedPerson" class="selected-person">
          <span class="memory-avatar large"><img v-if="avatar(selectedPerson)" :src="avatar(selectedPerson)" alt="" referrerpolicy="no-referrer" @error="badAvatars.add(selectedPerson.id)" /><span v-else>{{ initial(selectedPerson) }}</span></span>
          <div class="identity-text"><span class="selected-name">{{ selectedPerson.name }}</span><span class="identity-id mono">{{ selectedPerson.qq ? `QQ ${selectedPerson.qq}` : selectedPerson.id }}</span></div>
          <n-button v-if="selectedPerson.id !== 'self'" size="small" secondary class="relations-link" :loading="refreshingProfile" :disabled="refreshingProfile" @click="refreshProfile">整理画像</n-button>
          <n-button size="small" secondary @click="router.push({ path: '/relations', query: { person: selectedPerson.id } })">关系图谱</n-button>
        </div>
        <n-tabs v-if="selectedPerson || tab !== 'profile'" :value="tab" type="line" @update:value="changeTab">
          <n-tab name="profile">人物画像</n-tab>
          <n-tab name="memories">历史记忆</n-tab>
          <n-tab name="summaries">每日摘要</n-tab>
        </n-tabs>
        <div v-else class="portrait-heading"><span>人物画像</span><n-button text size="small" @click="changeTab('memories')">历史记录</n-button></div>

        <n-alert v-if="loadError" type="error" class="memory-alert" role="alert">{{ loadError }} <n-button text size="tiny" @click="loadCurrent">重试</n-button></n-alert>
        <n-alert v-if="profileNotice && tab === 'profile'" :type="profileRefreshFailed ? 'error' : 'info'" class="memory-alert">{{ profileNotice }}</n-alert>
        <n-spin :show="loading || (!selectedPerson && peopleLoading)">
          <div v-if="tab === 'profile'" class="memory-records">
            <article v-if="selectedPerson && profile.length" class="portrait-record">
              <dl class="portrait-fields"><template v-for="fact in profile" :key="fact.field"><dt>{{ profileLabels[fact.field] || fact.field }}</dt><dd>{{ fact.content }}</dd></template></dl>
              <n-collapse class="portrait-evidence">
                <n-collapse-item title="来源依据" name="evidence">
                  <div v-for="fact in profile" :key="fact.field" class="profile-evidence">
                    <span class="record-time">{{ profileLabels[fact.field] || fact.field }} · {{ fmtTs(fact.updated_at) }}</span>
                    <blockquote>{{ fact.evidence_quote }}</blockquote>
                  </div>
                </n-collapse-item>
              </n-collapse>
            </article>
            <div v-else-if="!selectedPerson" class="portrait-directory">
              <button v-for="person in people" :key="person.id" type="button" class="portrait-card" @click="choosePerson(person)">
                <span class="portrait-card-head"><span class="memory-avatar"><img v-if="avatar(person)" :src="avatar(person)" alt="" loading="lazy" referrerpolicy="no-referrer" @error="badAvatars.add(person.id)" /><span v-else>{{ initial(person) }}</span></span><span class="identity-text"><span class="identity-name">{{ person.name }}</span><span class="identity-id mono">{{ person.qq || person.id }}</span></span></span>
                <span v-if="person.profile?.length" class="portrait-preview">{{ person.profile.map(f => f.content).join('；') }}</span>
                <span v-else class="record-time">暂无画像</span>
                <time v-if="person.profile_updated_at" class="record-time">{{ fmtTs(person.profile_updated_at) }}</time>
              </button>
            </div>
            <EmptyState v-if="!loading && !peopleLoading && !loadError && !peopleError && (selectedPerson ? !profile.length : !people.length)" :title="selectedPerson ? '暂无人物画像' : '暂无匹配人物'" />
          </div>
          <div v-else class="memory-records">
            <article v-for="record in records" :key="record.id" class="memory-record">
              <button v-if="record.owner_type !== 'chat'" type="button" class="record-owner" @click="choosePerson(record.owner)">
                <span class="memory-avatar small"><img v-if="avatar(record.owner)" :src="avatar(record.owner)" alt="" loading="lazy" referrerpolicy="no-referrer" @error="badAvatars.add(record.owner.id)" /><span v-else>{{ initial(record.owner) }}</span></span>
                <span class="identity-name">{{ record.owner.name }}</span><span class="record-qq mono">{{ record.owner.qq || record.owner_id }}</span>
              </button>
              <div v-else class="record-owner"><span class="memory-avatar small"><YtIcon name="chat" :size="15" /></span><span class="identity-name">{{ record.conversation?.label || record.owner.name }}</span></div>
              <p class="memory-text">{{ record.content }}</p>
              <div class="record-meta">
                <n-tag size="tiny" :bordered="false">{{ sourceLabel(record) }}</n-tag>
                <span>{{ record.conversation?.label || '来源未记录' }}</span>
                <span v-if="record.conversation?.chat_type === 'group' && record.group_card && record.group_card !== record.owner.name">{{ record.group_card }}</span>
                <time class="record-time">{{ tab === 'summaries' ? record.date : fmtTs(record.updated_at) }}</time>
                <n-button v-if="record.source_msg_id && record.conversation" text size="tiny" type="primary" @click="showSource(record)">查看出处</n-button>
              </div>
            </article>
            <EmptyState v-if="!loading && !loadError && !records.length" :title="tab === 'summaries' ? '暂无摘要' : '暂无记忆'" />
          </div>
        </n-spin>
        <div v-if="tab !== 'profile'" class="memory-pagination">
          <span class="mono">共 {{ total }} 条</span>
          <n-pagination :page="page" :page-size="24" :item-count="total" :page-slot="3" :disabled="loading" @update:page="changePage" />
        </div>
      </n-card>
    </div>

    <n-drawer :show="sourceOpen" :width="520" :style="{ maxWidth: '100vw' }" @update:show="closeSource">
      <n-drawer-content :title="sourceData?.is_window ? '归纳出处' : '原消息'" closable>
        <n-alert v-if="sourceError" type="error" role="alert">{{ sourceError }} <n-button text size="tiny" @click="loadSource">重试</n-button></n-alert>
        <n-spin :show="sourceLoading">
          <div v-if="sourceData?.conversation" class="source-conversation">{{ sourceData.conversation.label }}</div>
          <article v-for="message in sourceData?.messages || []" :key="message.msg_id" class="source-message">
            <div class="source-author"><span>{{ message.name || message.person_id }}</span><time>{{ fmtTs(message.ts) }}</time></div>
            <p>{{ message.text || (message.has_image ? '图片消息' : '空消息') }}</p>
          </article>
          <EmptyState v-if="!sourceLoading && !sourceError && !sourceData?.messages?.length" title="来源未记录" />
          <span v-if="sourceData?.has_more" class="record-time">显示前 60 条消息</span>
        </n-spin>
      </n-drawer-content>
    </n-drawer>
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { api } from '../api'
import { fmtTs } from '../fmt'
import EmptyState from '../components/EmptyState.vue'
import YtIcon from '../components/YtIcon.vue'
import mascot from '../assets/mascot.png'

type Person = { id: string; name: string; qq?: string | null; avatar_url?: string | null; kind?: string; profile?: Profile[]; profile_updated_at?: number | null }
type Conversation = { id: string; chat_type: string | null; label: string }
type Memory = { id: number; owner_type: string; owner_id: string; owner: Person; content: string; source: string; updated_at: number; date?: string; source_msg_id?: number; source_end_msg_id?: number; conversation?: Conversation; group_card?: string }
type Profile = { field: string; content: string; evidence_quote: string; updated_at: number }
type Source = { conversation?: Conversation; is_window?: boolean; has_more?: boolean; messages: { msg_id: number; person_id: string; name?: string; text: string; ts: number; has_image: boolean }[] }
type Tab = 'memories' | 'summaries' | 'profile'
const router = useRouter()
const search = ref('')
const conversationKey = ref<string | null>(null)
const conversations = ref<Conversation[]>([])
const conversationsLoading = ref(false)
const conversationOptions = computed(() => conversations.value.map(c => ({ label: c.label, value: JSON.stringify([c.chat_type, c.id]) })))
const people = ref<Person[]>([])
const peopleTotal = ref(0)
const selectedPerson = ref<Person | null>(null)
const peoplePage = ref(1)
const peopleHasMore = ref(false)
const peopleLoading = ref(false)
const peopleError = ref('')
const tab = ref<Tab>('profile')
const records = ref<Memory[]>([])
const profile = ref<Profile[]>([])
const refreshingProfile = ref(false)
const profileNotice = ref('')
const profileRefreshFailed = ref(false)
const page = ref(1)
const total = ref(0)
const loading = ref(false)
const loadError = ref('')
const badAvatars = reactive(new Set<string>())
const sourceOpen = ref(false)
const sourceLoading = ref(false)
const sourceError = ref('')
const sourceData = ref<Source | null>(null)
let sourceRecord: { id: number; kind: string } | null = null
let listController: AbortController | undefined
let peopleController: AbortController | undefined
let conversationController: AbortController | undefined
let sourceController: AbortController | undefined
let refreshController: AbortController | undefined
let searchTimer: ReturnType<typeof setTimeout> | undefined
let disposed = false
const profileLabels: Record<string, string> = { preferred_name: '称呼', technical_preferences: '技术偏好', ongoing_projects: '进行中的项目', communication_style: '交流偏好', interests: '兴趣爱好' }
function avatar(p: Person) { return badAvatars.has(p.id) ? undefined : p.id === 'self' ? mascot : p.avatar_url || undefined }
function initial(p: Person) { return Array.from(p.name || '?')[0] }
function errorText(e: any) { return e.response?.data?.error || '加载失败，请重试' }
function sourceLabel(r: Memory) { return r.source_end_msg_id ? '窗口归纳' : r.source === 'explicit' && r.source_msg_id ? '消息记忆' : r.source === 'consolidation' ? '自动归纳' : '历史记忆' }
function query(includePerson = true) {
  const p = new URLSearchParams()
  if (includePerson && selectedPerson.value) p.set('person_id', selectedPerson.value.id)
  if (search.value.trim()) p.set('search', search.value.trim())
  if (tab.value !== 'profile' && conversationKey.value) { const [kind, id] = JSON.parse(conversationKey.value); p.set('chat_id', id); if (kind) p.set('chat_type', kind) }
  return p
}
async function loadPeople() {
  peopleController?.abort()
  const request = peopleController = new AbortController()
  peopleLoading.value = true; peopleError.value = ''
  const q = query(false); q.set('page', String(peoplePage.value)); q.set('page_size', '24')
  try {
    const { data } = await api.get(`/memories/people?${q}`, { signal: request.signal })
    if (request.signal.aborted || disposed) return
    people.value = data.people || []; peopleTotal.value = data.total || 0; peoplePage.value = data.page || 1; peopleHasMore.value = !!data.has_more
  } catch (e) { if (!request.signal.aborted && !disposed) peopleError.value = errorText(e) }
  finally { if (peopleController === request) peopleLoading.value = false }
}
async function loadConversations() {
  conversationController?.abort()
  const request = conversationController = new AbortController()
  conversationsLoading.value = true
  const q = new URLSearchParams()
  if (selectedPerson.value) q.set('person_id', selectedPerson.value.id)
  try {
    const { data } = await api.get(`/memories/conversations?${q}`, { signal: request.signal })
    if (request.signal.aborted || disposed) return
    conversations.value = data.conversations || []
    if (conversationKey.value && !conversationOptions.value.some(c => c.value === conversationKey.value)) conversationKey.value = null
  } catch (e) { if (!request.signal.aborted && !disposed) loadError.value = errorText(e) }
  finally { if (conversationController === request) conversationsLoading.value = false }
}
async function loadCurrent() {
  listController?.abort()
  const request = listController = new AbortController()
  loading.value = true; loadError.value = ''
  try {
    if (tab.value === 'profile') {
      if (selectedPerson.value) {
        const { data } = await api.get(`/relations/person/${encodeURIComponent(selectedPerson.value.id)}`, { signal: request.signal })
        if (!request.signal.aborted && !disposed) profile.value = data.profile || []
      }
    } else {
      const q = query(); q.set('page', String(page.value)); q.set('page_size', '24')
      const { data } = await api.get(`/${tab.value}?${q}`, { signal: request.signal })
      if (request.signal.aborted || disposed) return
      records.value = data[tab.value] || []; total.value = data.total || 0; page.value = data.page || 1
    }
  } catch (e) { if (!request.signal.aborted && !disposed) loadError.value = errorText(e) }
  finally { if (listController === request) loading.value = false }
}
function resetRecords() { page.value = 1; records.value = []; profile.value = []; closeSource(false); void loadCurrent() }
async function refreshProfile() {
  const id = selectedPerson.value?.id
  if (!id || refreshingProfile.value) return
  const request = refreshController = new AbortController()
  refreshingProfile.value = true; profileNotice.value = ''; profileRefreshFailed.value = false
  try {
    const { data } = await api.post(`/memories/people/${encodeURIComponent(id)}/refresh`, {}, { signal: request.signal, timeout: 120000 })
    if (disposed || request.signal.aborted || selectedPerson.value?.id !== id) return
    profileNotice.value = data.status === 'updated' ? '画像已更新' : data.status === 'unchanged' ? '近期已整理，请稍后重试' : '未找到可确认的新资料'
    void loadPeople()
    if (tab.value === 'profile') await loadCurrent()
  } catch (e) {
    if (!disposed && !request.signal.aborted && selectedPerson.value?.id === id) { profileNotice.value = errorText(e); profileRefreshFailed.value = true }
  } finally { refreshingProfile.value = false }
}
function choosePerson(person: Person | null) {
  if (selectedPerson.value?.id === person?.id && tab.value === 'profile') return
  selectedPerson.value = person
  profileNotice.value = ''; profileRefreshFailed.value = false
  tab.value = 'profile'
  void loadConversations(); resetRecords()
}
function changeTab(value: Tab) { tab.value = value; void loadPeople(); if (value !== 'profile') void loadConversations(); resetRecords() }
function changePage(value: number) { page.value = value; records.value = []; closeSource(false); void loadCurrent() }
function changePeoplePage(delta: number) { peoplePage.value += delta; void loadPeople() }
function refresh() { void loadPeople(); if (tab.value !== 'profile') void loadConversations(); resetRecords() }
watch(search, () => {
  listController?.abort(); peopleController?.abort(); closeSource(false); clearTimeout(searchTimer)
  searchTimer = setTimeout(() => { selectedPerson.value = null; if (tab.value !== 'profile') tab.value = 'memories'; peoplePage.value = 1; refresh() }, 250)
})
watch(conversationKey, () => { peoplePage.value = 1; void loadPeople(); resetRecords() })
async function loadSource() {
  if (!sourceRecord) return
  sourceController?.abort()
  const request = sourceController = new AbortController()
  sourceLoading.value = true; sourceError.value = ''; sourceData.value = null
  try {
    const { data } = await api.get(`/memories/${sourceRecord.id}/source?kind=${sourceRecord.kind}`, { signal: request.signal })
    if (!request.signal.aborted && !disposed) sourceData.value = data
  } catch (e) { if (!request.signal.aborted && !disposed) sourceError.value = errorText(e) }
  finally { if (sourceController === request) sourceLoading.value = false }
}
function showSource(record: Memory) { sourceRecord = { id: record.id, kind: tab.value === 'summaries' ? 'summary' : 'memory' }; sourceOpen.value = true; void loadSource() }
function closeSource(value: boolean) { sourceOpen.value = value; if (!value) { sourceController?.abort(); sourceData.value = null; sourceError.value = ''; sourceLoading.value = false } }
onMounted(refresh)
onBeforeUnmount(() => { disposed = true; clearTimeout(searchTimer); listController?.abort(); peopleController?.abort(); conversationController?.abort(); sourceController?.abort(); refreshController?.abort() })
</script>

<style scoped>
.memory-toolbar { display: grid; grid-template-columns: minmax(220px, 1fr) minmax(180px, 300px); gap: 12px; }
.memory-toolbar.profile-toolbar { grid-template-columns: minmax(0, 1fr); }
.memory-layout { display: grid; grid-template-columns: 230px minmax(0, 1fr); gap: 18px; align-items: start; }
.people-panel, .memory-content { min-width: 0; }
.people-heading { color: var(--yt-ink-3); font-size: 12px; margin-bottom: 10px; }
.person-option { display: flex; width: 100%; gap: 10px; align-items: center; text-align: left; border: 0; border-radius: 8px; background: transparent; color: var(--yt-ink-2); padding: 10px 8px; margin-bottom: 4px; font: inherit; cursor: pointer; }
.person-option:hover { background: var(--yt-hover-bg); }
.person-option.active { background: var(--yt-active-bg); color: var(--yt-primary); }
.memory-avatar { display: inline-flex; flex: none; width: 36px; height: 36px; align-items: center; justify-content: center; overflow: hidden; border-radius: 50%; color: var(--yt-primary); background: var(--yt-active-bg); font-size: 15px; }
.memory-avatar img { width: 100%; height: 100%; object-fit: cover; }
.memory-avatar.small { width: 28px; height: 28px; font-size: 12px; }
.memory-avatar.large { width: 48px; height: 48px; font-size: 20px; }
.identity-text { display: flex; flex-direction: column; min-width: 0; gap: 2px; }
.identity-name { font-weight: 600; overflow-wrap: anywhere; }
.identity-id, .record-qq { font-size: 12px; color: var(--yt-ink-3); overflow-wrap: anywhere; }
.selected-person { display: flex; align-items: center; gap: 12px; padding-bottom: 18px; }
.selected-name { font-size: 18px; font-weight: 600; overflow-wrap: anywhere; }
.relations-link { margin-left: auto; }
.memory-records { min-height: 220px; }
.portrait-heading { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 4px 0 16px; font-weight: 600; border-bottom: 1px solid var(--yt-card-border); }
.portrait-directory { display: grid; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); gap: 14px; padding-top: 18px; }
.portrait-card { display: flex; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--yt-card-border); border-radius: 10px; background: var(--yt-elevated); color: var(--yt-ink-1); text-align: left; font: inherit; cursor: pointer; min-width: 0; }
.portrait-card:hover { border-color: var(--yt-primary); }
.portrait-card-head { display: flex; align-items: center; gap: 10px; }
.portrait-preview { line-height: 1.75; white-space: pre-wrap; overflow-wrap: anywhere; }
.portrait-record { padding-top: 20px; }
.portrait-fields { display: grid; grid-template-columns: 110px minmax(0, 1fr); gap: 18px 14px; margin: 0; }
.portrait-fields dt { color: var(--yt-ink-3); }
.portrait-fields dd { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--yt-ink-1); }
.portrait-evidence { margin-top: 24px; }
.profile-evidence { padding-bottom: 12px; }
.profile-evidence blockquote { margin: 8px 0; padding-left: 12px; border-left: 2px solid var(--yt-card-border); color: var(--yt-ink-3); overflow-wrap: anywhere; }
.memory-record { padding: 20px 0; border-bottom: 1px solid var(--yt-card-border); }
.memory-record:last-child { border-bottom: 0; }
.record-owner { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; border: 0; padding: 0; background: transparent; color: var(--yt-ink-2); font: inherit; text-align: left; }
button.record-owner { cursor: pointer; }
.memory-text { color: var(--yt-ink-1); white-space: pre-wrap; overflow-wrap: anywhere; margin: 10px 0; line-height: 1.75; }
.record-meta { display: flex; gap: 8px 12px; align-items: center; flex-wrap: wrap; color: var(--yt-ink-3); font-size: 12px; }
.record-time { color: var(--yt-ink-3); font-size: 12px; }
.memory-pagination, .people-pagination { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 10px; padding-top: 16px; color: var(--yt-ink-3); font-size: 12px; }
.memory-alert { margin: 14px 0; }
.memory-empty-small { padding: 20px 0; color: var(--yt-ink-3); font-size: 12px; text-align: center; }
.source-conversation { color: var(--yt-ink-3); margin-bottom: 16px; }
.source-message { padding: 14px 0; border-bottom: 1px solid var(--yt-card-border); }
.source-author { display: flex; justify-content: space-between; gap: 12px; flex-wrap: wrap; color: var(--yt-ink-3); font-size: 12px; }
.source-message p { white-space: pre-wrap; overflow-wrap: anywhere; color: var(--yt-ink-1); }
@media (max-width: 760px) {
  .memory-layout { grid-template-columns: minmax(0, 1fr); }
  .memory-toolbar { grid-template-columns: minmax(0, 1fr); }
  .people-panel :deep(.n-card__content) { display: flex; flex-direction: column; }
  .people-panel :deep(.n-spin-content) { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 4px; }
  .person-option { min-width: 0; padding: 8px 5px; }
  .people-panel .identity-id { font-size: 11px; }
  .selected-person { flex-wrap: wrap; }
  .relations-link { margin-left: 0; }
  .memory-pagination { justify-content: center; }
  .portrait-directory { grid-template-columns: minmax(0, 1fr); }
  .portrait-fields { grid-template-columns: minmax(0, 1fr); gap: 8px; }
  .portrait-fields dd { margin-bottom: 12px; }
}
</style>
