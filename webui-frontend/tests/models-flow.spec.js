import { expect, test } from '@playwright/test'

async function openModels(page, { saveError = false, staleReadback = false, testGate, loadGate, loadError = false } = {}) {
  let failNextLoad = loadError
  let stored = {
    provider: {
      newapi: {
        base_url: 'https://mock.invalid/v1', api_key_env: '***', api_key_present: true,
        models: ['space-bunny-free', 'kimi-k3', 'mimo-v2.6-flash-free'],
      },
    },
    roles: Object.fromEntries(['decision', 'bot_chat', 'agent_exec'].map(role =>
      [role, { provider: 'newapi', model: 'space-bunny-free' }])),
  }
  const calls = []
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', async route => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    if (path === '/api/config') {
      if (request.method() === 'GET') {
        calls.push('read')
        if (loadGate) await loadGate
        if (failNextLoad) {
          failNextLoad = false
          return route.fulfill({ status: 503, json: { error: '模拟读取失败' } })
        }
        return route.fulfill({ json: { providers: stored } })
      }
      calls.push('save')
      if (saveError) return route.fulfill({ status: 400, json: { error: '模拟保存失败' } })
      if (!staleReadback) {
        stored = structuredClone(request.postDataJSON().providers)
        for (const provider of Object.values(stored.provider)) {
          provider.api_key_env = '***'
          provider.api_key_present = true
        }
      }
      return route.fulfill({ json: { ok: true, applied: ['LLM Provider 重建热应用'] } })
    }
    if (path === '/api/llm/test') {
      const binding = stored.roles[request.postDataJSON().role]
      const model = binding.model
      calls.push(`test:${model}`)
      if (testGate) await testGate
      return route.fulfill({ json: { ok: true, model, latency_ms: 42, reasoning_effort: binding.reasoning_effort ?? null } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/models')
  const card = page.locator('.role-card').filter({ has: page.getByText('bot_chat', { exact: true }) })
  if (!loadGate && !loadError) await expect(card.getByRole('button', { name: '保存并测试' })).toBeEnabled()
  return { card, calls, stored: () => stored }
}

test('slow configuration shows loading instead of empty providers or unbound roles', async ({ page }) => {
  let release
  const loadGate = new Promise(resolve => { release = resolve })
  const { card } = await openModels(page, { loadGate })
  await expect(page.getByRole('status')).toContainText('正在加载模型配置')
  await expect(page.getByText('尚未添加模型服务商')).toHaveCount(0)
  await expect(page.getByText('未绑定', { exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '保存配置', exact: true })).toBeDisabled()
  await page.getByRole('button', { name: '切换主题' }).click()
  release()
  await expect(card.getByRole('button', { name: '保存并测试' })).toBeEnabled()
  await expect(page.getByRole('status')).toHaveCount(0)
})

test('failed configuration loads can be retried without showing an empty editor', async ({ page }) => {
  const { card } = await openModels(page, { loadError: true })
  await expect(page.getByRole('alert')).toContainText('模拟读取失败')
  await expect(page.locator('.role-card')).toHaveCount(0)
  await expect(page.getByText('尚未添加模型服务商')).toHaveCount(0)
  await page.getByRole('button', { name: '重新加载', exact: true }).click()
  await expect(card.getByRole('button', { name: '保存并测试' })).toBeEnabled()
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('recommended reasoning persists independently per role and is used by save and test', async ({ page }) => {
  const { card, stored } = await openModels(page)
  const decision = page.locator('.role-card').filter({ has: page.getByText('decision', { exact: true }) })
  const agent = page.locator('.role-card').filter({ has: page.getByText('agent_exec', { exact: true }) })
  await page.getByRole('button', { name: '决策 low / 回复 high' }).click()
  await expect(decision.locator('.n-select').nth(2)).toContainText('low')
  await expect(card.locator('.n-select').nth(2)).toContainText('high')
  await expect(agent.locator('.n-select').nth(2)).toContainText('服务商默认')
  await card.getByRole('button', { name: '保存并测试' }).click()
  await expect(card.getByText('space-bunny-free · high · 42ms')).toBeVisible()
  expect(stored().roles.decision.reasoning_effort).toBe('low')
  expect(stored().roles.bot_chat.reasoning_effort).toBe('high')
  expect(stored().roles.agent_exec.reasoning_effort).toBeUndefined()
  await page.reload()
  await expect(card.locator('.n-select').nth(2)).toContainText('high')
  await expect(decision.locator('.n-select').nth(2)).toContainText('low')
  await expect(page.getByText('有未保存更改')).toHaveCount(0)
})

test('reasoning-only readback mismatch prevents testing an outdated binding', async ({ page }) => {
  const { card, calls } = await openModels(page, { staleReadback: true })
  await page.getByRole('button', { name: '决策 low / 回复 high' }).click()
  calls.length = 0
  await card.getByRole('button', { name: '保存并测试' }).click()
  await expect(card.getByText('保存后回读的模型配置不一致，请重试')).toBeVisible()
  expect(calls).toEqual(['save', 'read'])
})

async function selectModel(page, card, model) {
  await card.locator('.n-select').nth(1).click()
  await page.locator('.n-base-select-option').filter({ hasText: new RegExp(`^${model}$`) }).click()
}

test('save and test uses the selected model and survives page reload', async ({ page }) => {
  const { card, calls } = await openModels(page)
  await selectModel(page, card, 'kimi-k3')
  await expect(page.getByText('有未保存更改')).toBeVisible()
  calls.length = 0
  await card.getByRole('button', { name: '保存并测试' }).click()
  await expect(card.getByText('kimi-k3 · 42ms')).toBeVisible()
  expect(calls).toEqual(['save', 'read', 'test:kimi-k3'])
  await expect(page.getByText('有未保存更改')).toHaveCount(0)
  await selectModel(page, card, 'mimo-v2.6-flash-free')
  await expect(card.getByText('kimi-k3 · 42ms')).toHaveCount(0)
  await page.getByRole('button', { name: '刷新', exact: true }).click()
  await expect(card.locator('.n-select').nth(1)).toContainText('kimi-k3')
  await page.reload()
  await expect(card.locator('.n-select').nth(1)).toContainText('kimi-k3')
  await expect(page.getByText('有未保存更改')).toHaveCount(0)
})

test('save failure keeps the draft and never tests the old model', async ({ page }) => {
  const { card, calls } = await openModels(page, { saveError: true })
  await selectModel(page, card, 'kimi-k3')
  calls.length = 0
  await card.getByRole('button', { name: '保存并测试' }).click()
  await expect(card.getByText('模拟保存失败')).toBeVisible()
  expect(calls).toEqual(['save'])
  await expect(card.locator('.n-select').nth(1)).toContainText('kimi-k3')
  await expect(page.getByText('有未保存更改')).toBeVisible()
})

test('mismatched saved state stops the test', async ({ page }) => {
  const { card, calls } = await openModels(page, { staleReadback: true })
  await selectModel(page, card, 'kimi-k3')
  calls.length = 0
  await card.getByRole('button', { name: '保存并测试' }).click()
  await expect(card.getByText('保存后回读的模型配置不一致，请重试')).toBeVisible()
  expect(calls).toEqual(['save', 'read'])
})

test('late response cannot overwrite results after changing the selection', async ({ page }) => {
  let release
  const testGate = new Promise(resolve => { release = resolve })
  const { card, calls } = await openModels(page, { testGate })
  await selectModel(page, card, 'kimi-k3')
  await card.getByRole('button', { name: '保存并测试' }).click()
  await expect.poll(() => calls.includes('test:kimi-k3')).toBe(true)
  await selectModel(page, card, 'mimo-v2.6-flash-free')
  const finished = page.waitForResponse('**/api/llm/test')
  release()
  await finished
  await expect(card.locator('.test-badge')).toHaveCount(0)
  await expect(page.getByText('有未保存更改')).toBeVisible()
})
