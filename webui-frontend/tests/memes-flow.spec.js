import { expect, test } from '@playwright/test'
import fs from 'node:fs'
import path from 'node:path'

const artifacts = path.resolve('../data/artifacts/meme-gallery')
fs.mkdirSync(artifacts, { recursive: true })
const preview = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="240" height="180" viewBox="0 0 240 180"><rect width="240" height="180" rx="16" fill="#eff6ff"/><circle cx="120" cy="80" r="48" fill="#facc15"/><circle cx="104" cy="72" r="5" fill="#334155"/><circle cx="136" cy="72" r="5" fill="#334155"/><path d="M100 91 Q120 111 140 91" fill="none" stroke="#334155" stroke-width="5" stroke-linecap="round"/><text x="120" y="153" text-anchor="middle" font-family="sans-serif" font-size="16" fill="#2563eb">Gallery preview</text></svg>')

async function openGallery(page, options = {}) {
  const state = {
    items: Array.from({ length: 30 }, (_, i) => ({ id: i + 1, file: i < 26 ? `_inbox/待审${i + 1}.png` : `开心/开心${i + 1}.png`, category: i < 26 ? '_inbox' : '开心', status: i < 26 ? 'pending' : 'active', added_by: 'steal', use_count: i, created_at: i })),
    reads: [], writes: [], images: [], failLoad: options.failLoad, failWrite: options.failWrite, failImage: options.failImage, partial: options.partial,
  }
  await page.addInitScript(() => {
    localStorage.setItem('yt_token', 'test-only')
    window.galleryUrls = { created: [], revoked: [] }
    const create = URL.createObjectURL.bind(URL)
    const revoke = URL.revokeObjectURL.bind(URL)
    URL.createObjectURL = blob => { const url = create(blob); window.galleryUrls.created.push(url); return url }
    URL.revokeObjectURL = url => { window.galleryUrls.revoked.push(url); revoke(url) }
  })
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', async route => {
    const request = route.request()
    const url = new URL(request.url())
    if (url.pathname === '/api/memes') {
      state.reads.push(url.searchParams)
      if (state.failLoad) return route.fulfill({ status: 500, json: { error: '数据库暂时不可用' } })
      const status = url.searchParams.get('status')
      const category = url.searchParams.get('category')
      const search = url.searchParams.get('search') || ''
      const matching = state.items.filter(m => m.status === status && (!category || m.category === category) && (!search || m.file.includes(search) || m.category.includes(search) || String(m.id) === search))
      const sort = url.searchParams.get('sort')
      matching.sort((a, b) => sort === 'oldest' ? a.id - b.id : sort === 'popular' ? b.use_count - a.use_count : b.id - a.id)
      const pageSize = Number(url.searchParams.get('page_size'))
      const currentPage = Math.min(Number(url.searchParams.get('page')), Math.max(1, Math.ceil(matching.length / pageSize)))
      return route.fulfill({ json: {
        memes: matching.slice((currentPage - 1) * pageSize, currentPage * pageSize), total: matching.length, page: currentPage, page_size: pageSize,
        counts: { pending: state.items.filter(m => m.status === 'pending').length, active: state.items.filter(m => m.status === 'active').length },
        categories: [...new Set(state.items.filter(m => m.status === status).map(m => m.category))],
      } })
    }
    if (url.pathname.startsWith('/api/meme-file/')) {
      state.images.push(Number(url.pathname.split('/').pop()))
      if (state.failImage) return route.fulfill({ status: 404, json: { error: '文件不存在' } })
      return route.fulfill({ contentType: 'image/svg+xml', body: preview })
    }
    if (url.pathname.startsWith('/api/memes/')) {
      const body = request.postDataJSON()
      const batch = url.pathname.endsWith('/batch')
      const action = batch ? body.action : url.pathname.split('/').pop()
      const ids = batch ? body.ids : [Number(url.pathname.split('/')[3])]
      state.writes.push({ ids, action, category: body?.category })
      if (state.failWrite) return route.fulfill({ status: 500, json: { error: '保存失败，稍后重试' } })
      const updated = []
      const failed = []
      for (const id of ids) {
        const item = state.items.find(m => m.id === id)
        if (state.partial && id === ids[0]) { failed.push({ id, error: '文件无法删除' }); continue }
        if (action === 'reject') state.items = state.items.filter(m => m.id !== id)
        else { item.category = body?.category || 'misc'; if (action === 'approve') item.status = 'active' }
        updated.push(id)
      }
      return route.fulfill({ json: batch ? { ok: !failed.length, updated, failed } : { ok: true } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/meme')
  if (!state.failLoad) await expect(page.locator('.meme-card')).toHaveCount(24)
  return state
}

function card(page, id) { return page.locator('.meme-card').filter({ has: page.getByText(`#${id}`, { exact: true }) }) }

test('pagination, search, category and usage sorting with visible previews', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 })
  const state = await openGallery(page)
  await expect(card(page, 26).locator('img')).toBeVisible()
  expect(state.images.length).toBeLessThanOrEqual(24)
  await page.screenshot({ path: path.join(artifacts, 'desktop.png'), fullPage: true })
  await page.locator('.n-pagination-item').filter({ hasText: /^2$/ }).click()
  await expect(page.locator('.meme-card')).toHaveCount(2)
  await page.getByPlaceholder('搜索文件名、分类或编号').fill('待审26')
  await expect(page.locator('.meme-card')).toHaveCount(1)
  await expect(card(page, 26)).toBeVisible()
  expect(state.reads.at(-1).get('page')).toBe('1')
  await page.getByPlaceholder('搜索文件名、分类或编号').fill('不存在')
  await expect(page.getByText('没有匹配的表情包')).toBeVisible()
  await page.getByPlaceholder('搜索文件名、分类或编号').fill('')
  await page.getByText('已入库 (4)', { exact: true }).click()
  await expect(page.locator('.meme-card')).toHaveCount(4)
  await page.locator('.meme-filter .n-select').first().click()
  await page.getByText('开心', { exact: true }).last().click()
  await expect.poll(() => state.reads.at(-1).get('category')).toBe('开心')
  await page.locator('.meme-filter .n-select').last().click()
  await page.getByText('使用最多', { exact: true }).click()
  await expect.poll(() => state.reads.at(-1).get('sort')).toBe('popular')
  await expect(page.locator('.meme-card').first()).toContainText('#30')
})

test('bulk approval, recategorization and confirmed deletion', async ({ page }) => {
  const state = await openGallery(page)
  await page.getByText('选择本页', { exact: true }).click()
  await page.getByPlaceholder('统一分类（可选）').fill('生气')
  await page.getByRole('button', { name: '批量入库', exact: true }).click()
  await expect(page.locator('.meme-card')).toHaveCount(2)
  expect(state.writes[0].ids).toHaveLength(24)
  expect(state.writes[0].category).toBe('生气')
  await expect(page.getByRole('status')).toContainText('已处理 24 张')
  await page.getByText('已入库 (28)', { exact: true }).click()
  await expect(card(page, 30)).toBeVisible()
  await page.getByRole('textbox', { name: '表情包 30 分类' }).fill('开心新版')
  await card(page, 30).getByRole('button', { name: '保存分类' }).click()
  await expect(card(page, 30).getByText('开心新版', { exact: true })).toBeVisible()
  await card(page, 30).getByRole('checkbox').check()
  await card(page, 29).getByRole('checkbox').check()
  await page.getByPlaceholder('统一分类', { exact: true }).fill('下班')
  await page.getByRole('button', { name: '批量改分类' }).click()
  await expect(card(page, 30).getByText('下班', { exact: true })).toBeVisible()
  await card(page, 30).getByRole('button', { name: '删除', exact: true }).click()
  expect(state.items.some(m => m.id === 30)).toBe(true)
  await page.getByRole('button', { name: '取消', exact: true }).click()
  await card(page, 30).getByRole('button', { name: '删除', exact: true }).click()
  await page.getByRole('button', { name: '确认删除', exact: true }).click()
  await expect(card(page, 30)).toHaveCount(0)
  expect(state.items.some(m => m.id === 30)).toBe(false)
})

test('failed writes and refresh preserve category drafts', async ({ page }) => {
  const state = await openGallery(page, { failWrite: true })
  const input = page.getByRole('textbox', { name: '表情包 26 分类' })
  await input.fill('尚未保存')
  await page.getByRole('button', { name: '刷新', exact: true }).click()
  await expect(input).toHaveValue('尚未保存')
  await card(page, 26).getByRole('button', { name: '入库', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('保存失败，稍后重试')
  await expect(input).toHaveValue('尚未保存')
  await expect(card(page, 26).getByRole('button', { name: '入库', exact: true })).toBeEnabled()
  state.failWrite = false
  await card(page, 26).getByRole('button', { name: '入库', exact: true }).click()
  await expect(card(page, 26)).toHaveCount(0)
})

test('partial batch failures retain failed selections', async ({ page }) => {
  const state = await openGallery(page, { partial: true })
  await card(page, 26).getByRole('checkbox').check()
  await card(page, 25).getByRole('checkbox').check()
  await page.getByRole('button', { name: '批量删除' }).click()
  await page.getByRole('button', { name: '确认删除', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('已处理 1 张，1 张失败')
  await expect(card(page, 26).getByRole('checkbox')).toBeChecked()
  await expect(card(page, 25)).toHaveCount(0)
  expect(state.writes).toHaveLength(1)
})

test('load and image failures can be retried, mobile layout fits and object URLs are released', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 })
  const state = await openGallery(page, { failLoad: true, failImage: true })
  await expect(page.getByRole('alert')).toContainText('数据库暂时不可用')
  await expect(page.getByText('暂无待审表情包')).toHaveCount(0)
  state.failLoad = false
  await page.getByRole('button', { name: '重试加载' }).click()
  await expect(card(page, 26).getByText('图片加载失败')).toBeVisible()
  state.failImage = false
  await card(page, 26).getByRole('button', { name: '重试图片' }).click()
  await expect(card(page, 26).locator('img')).toBeVisible()
  expect(await page.locator('.meme-page').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: path.join(artifacts, 'mobile.png'), fullPage: true })
  await page.getByRole('button', { name: '切换主题' }).click()
  await page.screenshot({ path: path.join(artifacts, 'mobile-dark.png'), fullPage: true })
  await page.getByPlaceholder('搜索文件名、分类或编号').fill('不存在')
  await expect(page.getByText('没有匹配的表情包')).toBeVisible()
  await expect.poll(() => page.evaluate(() => window.galleryUrls.created.every(url => window.galleryUrls.revoked.includes(url)))).toBe(true)
})
