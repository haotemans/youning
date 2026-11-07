import { expect, test } from '@playwright/test'

test('task controls expose waiting, delivery and errors without repeating a rejected action', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  let phase = 'executing'
  const writes = []
  await page.route('**/api/**', async route => {
    const url = new URL(route.request().url())
    if (url.pathname === '/api/tasks') return route.fulfill({ json: { tasks: [{ task_id: 't1', goal: '整理资料', state: 'running', used_calls: 2, budget_max_calls: 8 }] } })
    if (url.pathname.endsWith('/events')) return route.fulfill({ json: { events: [] } })
    if (url.pathname.endsWith('/runtime')) return route.fulfill({ json: { runtime: { phase, delivery_state: 'unknown', stop_reason: '发送结果待核实' }, invocations: [{ id: 'i1', tool: 'search', state: 'succeeded', started_at: 100 }] } })
    if (url.pathname.endsWith('/control')) {
      const body = route.request().postDataJSON()
      writes.push(body.action)
      if (body.action === 'cancel') return route.fulfill({ status: 400, json: { error: '外部动作结果未确认' } })
      phase = body.action === 'pause' ? 'paused' : 'executing'
      return route.fulfill({ json: { ok: true } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/tasks')
  await expect(page.getByText('送达待核实', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '暂停', exact: true }).click()
  await expect(page.getByText('已暂停', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '继续', exact: true }).click()
  await expect(page.getByText('执行中', { exact: true })).toBeVisible()
  await page.getByRole('button', { name: '取消任务', exact: true }).click()
  await expect(page.getByText('外部动作结果未确认', { exact: true })).toBeVisible()
  expect(writes).toEqual(['pause', 'resume', 'cancel'])
})
