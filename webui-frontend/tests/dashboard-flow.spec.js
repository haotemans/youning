import { expect, test } from '@playwright/test'
import path from 'node:path'
import fs from 'node:fs'

test('today and cumulative model consumption remain distinct on desktop and mobile', async ({ page }) => {
  const artifacts = path.resolve('../data/artifacts/chat-fix-20261016')
  fs.mkdirSync(artifacts, { recursive: true })
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', route => {
    const pathname = new URL(route.request().url()).pathname
    return route.fulfill({ json: pathname === '/api/dashboard' ? {
      uptime_secs: 405, adapter_connected: true, mood: 'calm',
      messages_in_today: 12, messages_out_today: 3, decision_calls_today: 5, active_tasks: 0,
      tokens_today: 300, tokens_in_today: 200, tokens_out_today: 100, llm_calls_today: 3,
      tokens_total: 900, tokens_in_total: 600, tokens_out_total: 300, llm_calls_total: 9,
      mem_rss_bytes: 33554432, mem_total_bytes: 1073741824, cpu_percent: 1.5,
    } : pathname === '/api/events' ? { events: [] } : {} })
  })
  await page.setViewportSize({ width: 1440, height: 1100 })
  await page.goto('/')
  const today = page.locator('.resource-block').filter({ hasText: '今日 Token 总量' })
  const total = page.locator('.resource-block').filter({ hasText: '累计 Token 总量' })
  await expect(today.locator('.resource-value')).toHaveText('300')
  await expect(total.locator('.resource-value')).toHaveText('900')
  await expect(total.locator('.resource-cap')).toHaveText('9 次调用')
  await expect(total.locator('.token-item .v')).toHaveText(['600', '300'])
  await page.screenshot({ path: path.join(artifacts, 'dashboard-desktop.png'), fullPage: true })
  await page.setViewportSize({ width: 390, height: 844 })
  await expect(total).toBeVisible()
  expect(await page.locator('.resource-card').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: path.join(artifacts, 'dashboard-mobile.png'), fullPage: true })
})
