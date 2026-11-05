import { expect, test } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'

const artifacts = path.resolve('../data/artifacts/memories')
fs.mkdirSync(artifacts, { recursive: true })
const people = [
  { id: 'p_101', name: '小冰猪', qq: '101', kind: 'person', avatar_url: 'https://q1.qlogo.cn/g?b=qq&nk=101&s=100', profile_updated_at: 1700000000,
    profile: [{ field: 'technical_preferences', content: '长期使用 Rust', evidence_quote: '我长期使用 Rust', source_msg_id: 1, updated_at: 1700000000 },
      { field: 'ongoing_projects', content: '正在训练 LoRA', evidence_quote: '我在训练 LoRA', source_msg_id: 3, updated_at: 1700000000 }] },
  { id: 'p_202', name: 'SkyLoong', qq: '202', kind: 'person', avatar_url: 'https://q1.qlogo.cn/g?b=qq&nk=202&s=100', profile_updated_at: 1700000000,
    profile: [{ field: 'technical_preferences', content: '使用 Python 测试模型', evidence_quote: '我用 Python 测试模型', source_msg_id: 4, updated_at: 1700000000 }] },
]
const conversations = [
  { id: '100', chat_type: 'group', label: '群聊 100' }, { id: '200', chat_type: 'group', label: '群聊 200' },
  { id: '101', chat_type: 'private', label: '私聊 101' },
]
async function openMemories(page, options = {}) {
  const state = { requests: [], fail: !!options.fail, slowPerson: options.slowPerson, refreshed: false, refreshFail: false, rows: Array.from({ length: 251 }, (_, i) => {
    const person = i % 2 ? people[1] : people[0]
    return { id: i + 1, owner_id: person.id, owner_type: 'person', owner: person, content: `${person.name} 的训练记录 ${i + 1}`,
      source: 'consolidation', updated_at: 1700000000 + i, source_msg_id: 1, source_end_msg_id: 3, conversation: conversations[i % 3] }
  }) }
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('https://q1.qlogo.cn/**', route => route.abort())
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url()), q = url.searchParams
    state.requests.push({ path: url.pathname, person: q.get('person_id'), chat: q.get('chat_id'), page: q.get('page') })
    if (/\/api\/memories\/people\/[^/]+\/refresh$/.test(url.pathname)) {
      expect(route.request().method()).toBe('POST')
      if (state.refreshFail) return route.fulfill({ status: 503, json: { error: '未配置 Decision 模型' } })
      state.refreshed = true
      return route.fulfill({ json: { status: 'updated', updated_fields: 1, inspected_messages: 80 } })
    }
    if (url.pathname === '/api/memories/people') {
      if (state.fail) return route.fulfill({ status: 500, json: { error: '读取失败' } })
      const term = (q.get('search') || '').toLowerCase()
      const found = people.filter(p => !term || [p.name, p.qq, ...p.profile.map(f => f.content)].join(' ').toLowerCase().includes(term))
      return route.fulfill({ json: { people: found, page: 1, total: found.length, has_more: false } })
    }
    if (url.pathname === '/api/memories/conversations') return route.fulfill({ json: { conversations } })
    if (url.pathname.endsWith('/activity')) return route.fulfill({ json: { today_count: 11, last_seen: 1700000100, pending: !state.refreshed, profile_refresh: state.refreshed ? { completed_at: 1700000200, status: 'updated' } : {} } })
    if (/\/api\/memories\/people\/[^/]+\/messages$/.test(url.pathname)) {
      const id = url.pathname.split('/').at(-2)
      if (id === state.slowPerson) await new Promise(resolve => setTimeout(resolve, 450))
      if (state.fail) return route.fulfill({ status: 500, json: { error: '发言读取失败' } })
      let rows = Array.from({ length: 27 }, (_, i) => ({ msg_id: i + 1, text: i === 0 ? '' : `${id} 的原始发言 ${i + 1}`, has_image: i === 0, ts: 1700000000 + i, conversation: conversations[i % 3] }))
      if (q.get('chat_id')) rows = rows.filter(m => m.conversation.id === q.get('chat_id'))
      if (q.get('search')) rows = rows.filter(m => m.text.includes(q.get('search')))
      const current = Number(q.get('page') || 1), total = rows.length
      return route.fulfill({ json: { messages: rows.slice((current - 1) * 24, current * 24), total, page: current, page_size: 24 } }).catch(() => {})
    }
    if (url.pathname.startsWith('/api/relations/person/')) {
      const id = url.pathname.split('/').pop()
      if (id === state.slowPerson) await new Promise(resolve => setTimeout(resolve, 450))
      return route.fulfill({ json: { profile: [...(people.find(p => p.id === id)?.profile || []), ...(state.refreshed && id === 'p_101' ? [{ field: 'interests', content: '喜欢玩明日方舟', evidence_quote: '我喜欢玩明日方舟', updated_at: 1700000000 }] : [])] } }).catch(() => {})
    }
    if (/\/api\/memories\/\d+\/source/.test(url.pathname)) return route.fulfill({ json: { is_window: true, conversation: conversations[0], messages: [{ msg_id: 1, person_id: 'p_101', name: '冰猪', text: '我长期使用 Rust', ts: 1700000000 }] } })
    if (url.pathname === '/api/memories' || url.pathname === '/api/summaries') {
      if (state.fail) return route.fulfill({ status: 500, json: { error: '读取失败' } })
      let rows = state.rows.filter(r => (!q.get('person_id') || r.owner_id === q.get('person_id')) && (!q.get('chat_id') || (r.conversation.id === q.get('chat_id') && r.conversation.chat_type === q.get('chat_type'))))
      const search = (q.get('search') || '').toLowerCase()
      rows = rows.filter(r => !search || [r.content, r.owner.name, r.owner.qq].join(' ').toLowerCase().includes(search))
      rows.sort((a, b) => b.id - a.id)
      const counts = { memories: rows.length, summaries: rows.length ? 1 : 0 }
      const summary = url.pathname === '/api/summaries'
      if (summary) rows = rows.length ? [{ ...rows[0], summary: state.summaryContent || '今日讨论训练参数', content: state.summaryContent || '今日讨论训练参数', date: '2026-11-04' }] : []
      const total = rows.length, size = Number(q.get('page_size') || 24), current = Math.min(Number(q.get('page') || 1), Math.max(1, Math.ceil(total / size)))
      if (q.get('person_id') === state.slowPerson) await new Promise(resolve => setTimeout(resolve, 450))
      return route.fulfill({ json: { [summary ? 'summaries' : 'memories']: rows.slice((current - 1) * size, current * size), total, counts, page: current, page_size: size } }).catch(() => {})
    }
    if (url.pathname === '/api/relations') return route.fulfill({ json: { nodes: people.map(p => ({ id: p.id, name: p.name, first_seen: 1, last_seen: 2 })), edges: [] } })
    return route.fulfill({ json: {} })
  })
  await page.goto('/memories')
  if (!state.fail) await expect(page.locator('.portrait-card')).toHaveCount(2)
  return state
}
const person = (page, id) => page.locator('.person-option').filter({ hasText: id })

test('portrait refresh is explicit, reloads the selected person and preserves the portrait on failure', async ({ page }) => {
  const state = await openMemories(page)
  await person(page, '101').click()
  await expect(page.locator('.portrait-record')).toContainText('长期使用 Rust')
  expect(state.requests.some(r => r.path.endsWith('/refresh'))).toBe(false)
  await page.getByRole('button', { name: '整理画像', exact: true }).click()
  await expect(page.locator('.portrait-record')).toContainText('兴趣爱好')
  await expect(page.locator('.portrait-record')).toContainText('喜欢玩明日方舟')
  state.refreshFail = true
  await page.getByRole('button', { name: '整理画像', exact: true }).click()
  await expect(page.getByRole('alert')).toHaveText('未配置 Decision 模型')
  await expect(page.locator('.portrait-record')).toContainText('长期使用 Rust')
})

test('real activity opens original speech with day, conversation, paging and retry controls', async ({ page }) => {
  const state = await openMemories(page)
  await person(page, '101').click()
  await expect(page.locator('.person-activity')).toContainText('今天 11 条发言')
  await expect(page.locator('.person-activity')).toContainText('尚未整理')
  expect(state.requests.some(r => r.path.endsWith('/messages'))).toBe(false)
  await page.getByRole('button', { name: '今天 11 条发言', exact: true }).click()
  await expect(page.locator('.message-record')).toHaveCount(24)
  await expect(page.locator('.message-records')).toContainText('图片消息')
  await expect(page.locator('.message-records')).toContainText('p_101 的原始发言')
  await page.locator('.memory-pagination .n-pagination-item').filter({ hasText: /^2$/ }).click()
  await expect(page.locator('.message-record')).toHaveCount(3)
  await page.getByRole('button', { name: '全部日期', exact: true }).click()
  await expect(page.getByLabel('发言日期')).toHaveValue('')
  await expect(page.locator('.message-record')).toHaveCount(24)
  await page.locator('.memory-toolbar .n-select').click()
  await page.locator('.n-base-select-menu').getByText('私聊 101', { exact: true }).click()
  await expect(page.locator('.message-record')).toHaveCount(9)
  await expect(page.locator('.message-records')).not.toContainText('群聊 100')
  state.fail = true
  await page.getByRole('button', { name: '刷新', exact: true }).click()
  await expect(page.getByRole('alert').filter({ hasText: '发言读取失败' })).toBeVisible()
  state.fail = false
  await page.getByRole('alert').filter({ hasText: '发言读取失败' }).getByRole('button', { name: '重试' }).click()
  await expect(page.locator('.message-record')).toHaveCount(9)
  await page.setViewportSize({ width: 390, height: 844 })
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true)
  await page.screenshot({ path: path.join(artifacts, 'messages-mobile.png'), fullPage: true })
})

test('switching people cancels old speech, and old window counts are labelled honestly', async ({ page }) => {
  const state = await openMemories(page, { slowPerson: 'p_101' })
  await person(page, '101').click()
  await page.getByText('原始发言', { exact: true }).click()
  await person(page, '202').click()
  await page.getByText('原始发言', { exact: true }).click()
  await expect(page.locator('.message-records')).toContainText('p_202 的原始发言')
  await page.waitForTimeout(550)
  await expect(page.locator('.message-records')).not.toContainText('p_101 的原始发言')
  // The old record remains stored; only its misleading display label is corrected.
  state.summaryContent = '今日发言 22 条。'
  await page.getByText('每日摘要', { exact: true }).click()
  await expect(page.locator('.memory-record')).toContainText('归纳窗口内发言 22 条。')
  await expect(page.locator('.memory-record')).not.toContainText('今日发言')
})

test('one portrait per person stays primary even with 251 history records', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 })
  const state = await openMemories(page)
  await expect(page.getByText('2 位人物', { exact: true })).toBeVisible()
  await expect(page.locator('.memory-record')).toHaveCount(0)
  expect(state.requests.some(r => ['/api/memories', '/api/summaries'].includes(r.path))).toBe(false)
  expect(state.requests.some(r => r.path.startsWith('/api/relations/person/'))).toBe(false)
  await expect(page.locator('.n-spin-body:visible')).toHaveCount(0)
  await page.screenshot({ path: path.join(artifacts, 'desktop.png'), fullPage: true, animations: 'disabled' })
  await page.getByRole('button', { name: '历史记录', exact: true }).click()
  await expect(page.locator('.memory-record')).toHaveCount(24)
  expect(state.requests.some(r => r.path === '/api/memories' && !r.person)).toBe(true)
  await page.getByRole('button', { name: '全部画像', exact: true }).click()
  await expect(page.locator('.portrait-card')).toHaveCount(2)
  const initialHistoryRequests = state.requests.filter(r => r.path === '/api/memories').length
  await person(page, '101').click()
  await expect(page.locator('.selected-name')).toHaveText('小冰猪')
  await expect(page.locator('.portrait-record')).toHaveCount(1)
  await expect(page.locator('.portrait-record')).toContainText('长期使用 Rust')
  await expect(page.locator('.portrait-record')).toContainText('正在训练 LoRA')
  await expect(page.locator('.memory-record')).toHaveCount(0)
  expect(state.requests.filter(r => r.path === '/api/memories').length).toBe(initialHistoryRequests)
  await page.screenshot({ path: path.join(artifacts, 'person.png'), fullPage: true })
  await page.getByText('来源依据', { exact: true }).click()
  await expect(page.locator('.profile-evidence blockquote').first()).toHaveText('我长期使用 Rust')
  await page.getByText('历史记忆', { exact: true }).click()
  await expect(page.locator('.memory-record')).toHaveCount(24)
  await expect(page.locator('.memory-record .identity-name').first()).toHaveText('小冰猪')
  await expect(page.locator('.memory-records')).toContainText('私聊 101')
  await expect(page.locator('.memory-records')).toContainText('群聊 100')
  await expect(page.locator('.memory-records')).toContainText('群聊 200')
  await page.locator('.memory-pagination .n-pagination-item').filter({ hasText: /^2$/ }).click()
  await expect.poll(() => state.requests.some(r => r.path === '/api/memories' && r.page === '2')).toBe(true)
  await person(page, '101').click()
  await expect(page.locator('.memory-toolbar .n-select')).toHaveCount(0)
  await expect(page.locator('.portrait-record')).toHaveCount(1)
  await expect(page.locator('.memory-record')).toHaveCount(0)
  await page.getByRole('button', { name: '关系图谱', exact: true }).click()
  await expect(page).toHaveURL(/\/relations\?person=p_101/)
  await expect(page.locator('.person-name')).toHaveText('小冰猪')
})

test('conversation filter keeps a unified person, and sources load only when opened', async ({ page }) => {
  const state = await openMemories(page)
  await person(page, '101').click()
  await page.getByText('历史记忆', { exact: true }).click()
  await page.locator('.memory-toolbar .n-select').click()
  await page.locator('.n-base-select-menu').getByText('群聊 100', { exact: true }).click()
  await expect.poll(() => state.requests.some(r => r.path === '/api/memories' && r.person === 'p_101' && r.chat === '100')).toBe(true)
  await expect(page.locator('.selected-name')).toHaveText('小冰猪')
  await expect(page.locator('.memory-records')).not.toContainText('私聊 101')
  expect(state.requests.some(r => r.path.endsWith('/source'))).toBe(false)
  await page.getByRole('button', { name: '查看出处' }).first().click()
  await expect(page.locator('.source-message')).toContainText('我长期使用 Rust')
  await expect(page.locator('.source-conversation')).toHaveText('群聊 100')
})

test('search, late-response isolation, retry and phone/dark layouts', async ({ page }) => {
  const errors = []; page.on('pageerror', e => errors.push(e.message))
  const state = await openMemories(page, { slowPerson: 'p_101' })
  await person(page, '101').click()
  await person(page, '202').click()
  await expect(page.locator('.selected-name')).toHaveText('SkyLoong')
  await expect(page.locator('.portrait-record')).toContainText('使用 Python 测试模型')
  await page.waitForTimeout(550)
  await expect(page.locator('.portrait-record')).toContainText('使用 Python 测试模型')
  await expect(page.locator('.portrait-record')).not.toContainText('长期使用 Rust')
  await page.getByPlaceholder('搜索昵称、QQ 或记忆内容').fill('不存在的词')
  await expect(page.locator('.memory-content').getByText('暂无匹配人物', { exact: true })).toBeVisible()
  await page.getByPlaceholder('搜索昵称、QQ 或记忆内容').fill('')
  await expect(page.locator('.portrait-card')).toHaveCount(2)
  state.fail = true
  await page.getByRole('button', { name: '刷新', exact: true }).click()
  await expect(page.getByRole('alert')).toContainText('读取失败')
  state.fail = false
  await page.getByRole('button', { name: '重试', exact: true }).last().click()
  await expect(page.locator('.portrait-card')).toHaveCount(2)
  await page.getByRole('button', { name: '切换主题' }).click()
  await expect(page.locator('html')).toHaveClass(/dark/)
  await expect(page.locator('.n-spin-body:visible')).toHaveCount(0)
  await page.screenshot({ path: path.join(artifacts, 'dark.png'), fullPage: true, animations: 'disabled' })
  await page.setViewportSize({ width: 390, height: 844 })
  await expect(page.locator('.memory-content')).toBeVisible()
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBe(true)
  await page.screenshot({ path: path.join(artifacts, 'mobile.png'), fullPage: true, animations: 'disabled' })
  expect(errors).toEqual([])
})
