import { expect, test } from '@playwright/test'

const nodes = Array.from({ length: 159 }, (_, i) => ({
  id: i ? `p_${100000 + i}` : 'self', name: i ? `人物${i}` : '云团',
  first_seen: 1700000000, last_seen: 1700000100,
}))
const edges = Array.from({ length: 247 }, (_, i) => ({
  from: nodes[i < 158 ? 0 : i - 157].id, to: nodes[i % 158 + 1].id,
  trust: 0.5, familiar: 0.4,
}))

async function openGraph(page, options = {}) {
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.addInitScript(() => {
    localStorage.setItem('yt_token', 'test-only')
    window.drawnPeople = new Set()
    window.graphDraws = 0
    window.remoteDraws = 0
    const drawImage = CanvasRenderingContext2D.prototype.drawImage
    CanvasRenderingContext2D.prototype.drawImage = function (...args) {
      window.graphDraws++
      if (String(args[0]?.src).startsWith('https://q1.qlogo.cn/')) window.remoteDraws++
      return drawImage.apply(this, args)
    }
    const fillText = CanvasRenderingContext2D.prototype.fillText
    CanvasRenderingContext2D.prototype.fillText = function (text, ...args) {
      if (String(text).startsWith('人物')) window.drawnPeople.add(text)
      return fillText.call(this, text, ...args)
    }
  })
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('https://q1.qlogo.cn/**', route => {
    if (options.slowAvatars) return
    if (options.avatars) return route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="80"><circle cx="40" cy="40" r="40" fill="#2563eb"/></svg>' })
    return route.abort()
  })
  await page.route('**/api/**', async route => {
    const pathname = new URL(route.request().url()).pathname
    if (pathname === '/api/dashboard' && options.slowDashboard) return
    if (pathname === '/api/relations') {
      if (options.delayRelations) await new Promise(resolve => setTimeout(resolve, 500))
      if (options.fail) return route.fulfill({ status: 500, json: { error: '数据库暂时不可用' } })
      return route.fulfill({ json: options.empty ? { nodes: [], edges: [] } : options.graph || { nodes, edges } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/relations')
  return errors
}

test('159 people render without remote avatars and remain interactive', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 })
  const avatarRequests = []
  page.on('request', request => { if (request.url().startsWith('https://q1.qlogo.cn/')) avatarRequests.push(request.url()) })
  const errors = await openGraph(page, { slowAvatars: true, slowDashboard: true })
  await expect(page.getByText('159 节点 · 247 关系')).toBeVisible()
  await expect.poll(() => page.evaluate(() => window.drawnPeople.size), { timeout: 8000 }).toBeGreaterThan(100)
  expect(errors).toEqual([])
  expect(avatarRequests.length).toBeLessThanOrEqual(4)
  await page.screenshot({ path: '../data/artifacts/relations-desktop.png', animations: 'disabled' })
  await page.locator('.n-select').click()
  await page.getByText('人物1', { exact: true }).click()
  await expect(page.locator('.person-name')).toHaveText('人物1')
  await page.getByRole('button', { name: '关闭详情' }).click()
  await page.getByRole('button', { name: '重置视图' }).click()
  await page.getByRole('button', { name: '切换主题' }).click()
  expect(errors).toEqual([])
})

test('successful remote avatars progressively replace local placeholders', async ({ page }) => {
  const errors = await openGraph(page, { avatars: true })
  await expect.poll(() => page.evaluate(() => window.remoteDraws)).toBeGreaterThan(0)
  expect(errors).toEqual([])
})

test('emoji and supplementary Unicode nicknames do not break avatar encoding', async ({ page }) => {
  const graph = {
    nodes: [
      { id: 'self', name: '云团' },
      { id: 'p_100001', name: '🌸小花' },
      { id: 'p_100002', name: '𠮷同学' },
      { id: 'p_100003', name: '\ud800损坏字符' },
      { id: 'p_100004', name: '<&特殊字符' },
    ],
    edges: [],
  }
  const errors = await openGraph(page, { graph })
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect.poll(() => page.evaluate(() => window.graphDraws)).toBeGreaterThan(4)
  await page.locator('.n-select').click()
  await page.getByText('🌸小花', { exact: true }).click()
  await expect(page.locator('.person-name')).toHaveText('🌸小花')
  await expect(page.locator('.person-avatar')).toHaveJSProperty('complete', true)
  await expect.poll(() => page.locator('.person-avatar').evaluate(img => img.naturalWidth)).toBeGreaterThan(0)
  expect(errors).toEqual([])
})

test('failed requests show an error and can be retried', async ({ page }) => {
  const options = { fail: true }
  const errors = await openGraph(page, options)
  await expect(page.getByRole('alert')).toContainText('数据库暂时不可用')
  options.fail = false
  await page.getByRole('button', { name: '重新加载' }).click()
  await expect.poll(() => page.evaluate(() => window.graphDraws)).toBeGreaterThan(100)
  await expect(page.getByRole('alert')).toHaveCount(0)
  expect(errors).toEqual([])
})

test('empty graphs show a clear empty state', async ({ page }) => {
  const errors = await openGraph(page, { empty: true })
  await expect(page.getByText('暂无关系图谱数据')).toBeVisible()
  await expect(page.getByRole('button', { name: '重置视图' })).toBeDisabled()
  expect(errors).toEqual([])
})

test('leaving while a request is pending does not initialize a detached canvas', async ({ page }) => {
  const errors = await openGraph(page, { delayRelations: true })
  await expect(page.getByRole('status')).toContainText('正在加载关系图谱')
  await page.evaluate(() => { document.querySelector('.yt-brand').click() })
  await expect(page).toHaveURL('/')
  // The request completes after navigation; the old component must stay disposed.
  await page.waitForTimeout(700)
  expect(errors).toEqual([])
})

test('1000 disconnected people render with reduced motion on a narrow screen', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' })
  await page.setViewportSize({ width: 390, height: 844 })
  const graph = {
    nodes: Array.from({ length: 1000 }, (_, i) => ({ id: `node_${i}`, name: `人物${i}` })),
    edges: [],
  }
  const errors = await openGraph(page, { graph })
  await expect.poll(() => page.evaluate(() => window.graphDraws), { timeout: 8000 }).toBeGreaterThan(900)
  expect(errors).toEqual([])
})
