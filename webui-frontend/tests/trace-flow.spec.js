import { expect, test } from '@playwright/test'

const policy = { max_failures: 3, max_pending_age_secs: 600, group_interval_secs: 30, periodic_concurrency: 2, max_batch_messages: 40, max_batch_chars: 10000, direct_mode: 'immediate' }
const counts = { pending: 3, deciding: 1, executing: 0, uncertain: 1, failed: 1, retry_waiting: 1 }
const pending = { msg_id: 21, chat_id: '42', chat_type: 'group', sender_pid: 'p_2001', sender_name: '小冰猪', text: '刚才的问题还有一条补充', ts: 1800000000, state: 'pending', retry_at: 4102444800, attempts: 2 }
const uncertain = { ...pending, msg_id: 22, sender_name: '猫猫', text: '上一次操作结果未知', state: 'uncertain', retry_at: 0 }
const stopped = { ...pending, state: 'failed', text: '已停止的旧消息', retry_at: 0, attempts: 11, error: '已达到自动判断失败上限' }
const events = [{ id: 3, kind: 'DecisionMade', ts: 1800000000, payload: { trigger: 'periodic', anchor_msg_id: 18, chat_id: '42', chat_type: 'group', sender_pid: 'p_2001', sender_name: '小冰猪', batch_message_ids: [16, 17, 18], action: 'reply', reason: '接上公开话题', reply_mode: 'social', policy: { suggested_action: 'reply', activity: { level: 'lively', human_messages_2min: 12, active_speakers_2min: 3 }, cooldown_secs: 45, notes: [] }, fallback: false } }, { id: 2, kind: 'DecisionMade', ts: 1800000000, payload: { trigger: 'immediate', anchor_msg_id: 15, chat_id: '42', chat_type: 'private', sender_name: '猫猫', action: 'ignore', reason: '模型暂时不可用', fallback: true } }]

async function setup(page, fail = false) {
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  const writes = []
  await page.route('**/api/**', async route => {
    const req = route.request(), url = new URL(req.url())
    if (req.method() === 'POST') writes.push(req.postDataJSON())
    if (url.pathname === '/api/decisions/status') {
      if (fail) return route.fulfill({ status: 500, json: { error: '暂时无法读取队列' } })
      const state = url.searchParams.get('state')
      return route.fulfill({ json: { policy, counts, messages: state === 'failed' ? [stopped] : state === 'uncertain' ? [uncertain] : state === 'pending' ? [pending] : [pending, uncertain] } })
    }
    if (url.pathname === '/api/events') return route.fulfill({ json: { events } })
    if (url.pathname === '/api/config') return route.fulfill({ json: req.method() === 'POST' ? { applied: ['reply'] } : { config: {} } })
    return route.fulfill({ json: {} })
  })
  return writes
}

test('source and queue show real author, batch and uncertainty without replay', async ({ page }) => {
  const writes = await setup(page)
  await page.setViewportSize({ width: 1440, height: 1000 })
  await page.goto('/trace')
  await expect(page.getByText('群聊 30 秒', { exact: true })).toBeVisible()
  await expect(page.getByText('消息 #18 · 群聊 42')).toBeVisible()
  await expect(page.locator('.decision-source')).toContainText('小冰猪')
  await expect(page.locator('.decision-source')).toContainText('群聊批次 · 3 条')
  await expect(page.locator('.trace-detail')).toContainText('自然接话')
  await expect(page.locator('.trace-detail')).toContainText('群聊活跃')
  await expect(page.locator('.trace-detail')).toContainText('近2分钟 12 条 · 3 人')
  await expect(page.locator('.trace-detail')).toContainText('参与间隔 45 秒')
  await page.screenshot({ path: 'test-results/trace-desktop.png', fullPage: true })
  await page.locator('.tl-item').nth(1).click()
  await expect(page.locator('.trace-detail')).toContainText('判断未完成')
  await expect(page.locator('.trace-detail')).toContainText('即时请求')
  await expect(page.locator('.trace-detail').getByText('保持安静', { exact: true })).toHaveCount(0)
  await page.getByRole('button', { name: '待判断 3' }).click()
  await expect(page.locator('.queue-list')).toContainText('小冰猪')
  await expect(page.locator('.queue-list')).toContainText('等待重试')
  await page.getByRole('button', { name: '待核实 1' }).click()
  await expect(page.locator('.queue-list')).toContainText('上一次操作结果未知')
  await expect(page.locator('.queue-list')).not.toContainText('刚才的问题还有一条补充')
  await expect(page.getByRole('button', { name: /重新执行|重试操作/ })).toHaveCount(0)
  await page.getByRole('button', { name: '已停止重试 1' }).click()
  await expect(page.locator('.queue-list')).toContainText('累计失败 11 次')
  await expect(page.locator('.queue-list')).toContainText('已达到自动判断失败上限')
  await expect(page.locator('.queue-list')).not.toContainText('后重试')
  await page.setViewportSize({ width: 390, height: 844 })
  await page.getByText('决策记录', { exact: true }).click()
  await page.locator('.tl-item').first().click()
  await expect(page.locator('.trace-detail')).toContainText('自然接话')
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: 'test-results/trace-mobile.png', fullPage: true })
  expect(writes).toEqual([])
})

test('status error does not invent an empty healthy queue', async ({ page }) => {
  await setup(page, true)
  await page.goto('/trace')
  await expect(page.getByText('暂时无法读取队列', { exact: true })).toBeVisible()
  await expect(page.getByRole('button', { name: '待判断 —' })).toBeVisible()
})

test('parameters display server policy without saving read-only fields', async ({ page }) => {
  const writes = await setup(page)
  await page.goto('/params')
  await expect(page.getByText('每 30 秒', { exact: true })).toBeVisible()
  await expect(page.getByText('即时处理', { exact: true })).toBeVisible()
  await expect(page.getByText('3 次', { exact: true })).toBeVisible()
  await expect(page.getByText('10 分钟', { exact: true })).toBeVisible()
  await expect(page.getByText('40 条 / 10000 字符', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '保存运行参数' }).click()
  await expect(page.getByText(/已保存/)).toBeVisible()
  expect(writes).toHaveLength(1)
  expect(writes[0].config.pipeline).toEqual({ per_chat_queue_cap: 32, self_msg_ids_cap: 512, decision_cost_per_min_init: 30 })
})
