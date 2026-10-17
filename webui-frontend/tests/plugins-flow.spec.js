import { expect, test } from '@playwright/test'
import path from 'node:path'
import fs from 'node:fs'

const artifacts = path.resolve('../data/artifacts/plugin-refine-20261015')
fs.mkdirSync(artifacts, { recursive: true })

function catalog() {
  return [
    ['nai-image', '琼华', 'NAI 生图与提示词优化', '通过 NAI 4.5 或 NAI 5 生成图片，支持模型选择、/image 命令和 Skill 调用', '齐风·著', '尚之以琼华乎而。', ['/image', '/画', '/prompt']],
    ['social-media', '采薇', '视频与图文解析下载', '解析 B站、抖音、小红书和 X 的视频、图文，发送后自动清理文件', '小雅·采薇', '采薇采薇，薇亦作止。', ['/下载', '/download']],
    ['anysearch', '采苓', '联网搜索与网页提取', '联网搜索、领域搜索与网页正文提取，保留来源链接', '唐风·采苓', '采苓采苓，首阳之巅。', ['/搜索', '/search']],
    ['moderator', '敬之', '撤回、禁言与群聊管理', '群管理员撤回消息、禁言和解除禁言，实时核验权限', '周颂·敬之', '敬之敬之，天维显思。', ['/撤回', '/禁言', '/解除禁言']],
  ].map(([name, display_name, purpose, description, poem, verse, commands]) => ({
    name, display_name, description, version: '0.1.0', enabled: true, loaded: true, updating: false, pending_reload: false,
    presentation: { purpose, poem, verse, examples: name === 'nai-image' ? ['/image -m nai5 -r 2:3 成年女性在图书馆阅读', '/prompt -m nai5 黑发御姐，夜晚街道'] : ['在群聊发送命令，或直接 @云团描述你的请求'] },
    skills: [{ name: name + '.generate', description: purpose, commands }], tools: [{ name: name + '.generate' }], data_dir: '/app/data/plugins/' + name,
    invocations: { active: 0, succeeded: 3, failed: 0, rejected: 0, cancelled: 0, last_status: 'succeeded', last_duration_ms: 1250 },
    config_fields: name === 'nai-image' ? [
      { key: 'api_key', label: 'API Key', kind: 'string', secret: true, has_value: true },
      { key: 'base_url', label: 'API 地址', kind: 'string', default: 'https://nai.rinko.ai' },
      { key: 'default_model', label: '默认模型', kind: 'choice', default: 'nai4.5', options: [{ value: 'nai4.5', label: 'NAI 4.5' }, { value: 'nai5', label: 'NAI 5' }] },
      { key: 'enabled_option', label: '启用提示词优化', kind: 'boolean', default: true },
    ] : [],
    settings: { enabled: true, allowed_chats: null, timeout_ms: 360000, max_concurrency: 1, max_output_bytes: 65536, config: name === 'nai-image' ? { base_url: 'https://nai.rinko.ai', default_model: 'nai4.5', enabled_option: true } : {} },
  }))
}

async function openPlugins(page, { deferred = false, error = false } = {}) {
  const state = { plugins: catalog(), reads: 0, writes: [], released: false }
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', async route => {
    const pathname = new URL(route.request().url()).pathname
    if (pathname === '/api/plugins/list') {
      state.reads++
      if (state.released) state.plugins[0].updating = false
      return route.fulfill({ json: { ok: true, plugins: state.plugins } })
    }
    if (['/api/plugins/config', '/api/plugins/toggle'].includes(pathname)) {
      const body = route.request().postDataJSON()
      state.writes.push(body)
      if (error) return route.fulfill({ status: 400, json: { error: 'API 地址必须是有效的 HTTP(S) 链接' } })
      const plugin = state.plugins.find(plugin => plugin.name === body.name)
      if (body.settings) plugin.settings = body.settings
      else { plugin.enabled = body.enabled; plugin.loaded = body.enabled }
      plugin.updating = deferred && !state.released
      return route.fulfill({ status: deferred ? 202 : 200, json: { ok: true, applying: deferred } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/plugins')
  await expect(page.getByRole('heading', { name: '琼华', exact: true })).toBeVisible()
  return state
}

async function openConfig(page) {
  await page.locator('.plugin-card').filter({ has: page.getByRole('heading', { name: '琼华', exact: true }) }).getByRole('button', { name: '配置与用法' }).click()
  await expect(page.getByRole('button', { name: '保存并应用' })).toBeVisible()
}

test('poetic names, functional detail, search and responsive layouts', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 1000 })
  await openPlugins(page)
  await expect(page.locator('.plugin-card h2')).toHaveText(['琼华', '采薇', '采苓', '敬之'])
  await page.screenshot({ path: path.join(artifacts, 'plugins-desktop.png'), fullPage: true, animations: 'disabled' })
  await page.getByRole('button', { name: '查看琼华详情' }).click()
  await expect(page.locator('.plugin-verse')).toContainText('齐风·著')
  await expect(page.getByText('/image -m nai5 -r 2:3 成年女性在图书馆阅读', { exact: true })).toBeVisible()
  await page.screenshot({ path: path.join(artifacts, 'plugins-detail.png'), fullPage: true, animations: 'disabled' })
  await page.keyboard.press('Escape')
  await page.getByPlaceholder('搜索名称或功能').fill('搜索')
  await expect(page.locator('.plugin-card')).toHaveCount(1)
  await page.getByPlaceholder('搜索名称或功能').fill('')
  await page.getByRole('button', { name: '切换主题' }).click()
  await page.screenshot({ path: path.join(artifacts, 'plugins-dark.png'), fullPage: true, animations: 'disabled' })
  await page.getByRole('button', { name: '切换主题' }).click()
  await page.setViewportSize({ width: 390, height: 844 })
  await expect(page.locator('.plugin-card')).toHaveCount(4)
  await expect(page.locator('.yt-sider')).toHaveCount(0)
  expect((await page.locator('.plugin-card').first().boundingBox()).width).toBeGreaterThan(330)
  expect(await page.locator('.plugins-page').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true)
  await page.screenshot({ path: path.join(artifacts, 'plugins-mobile.png'), fullPage: true, animations: 'disabled' })
  await page.getByRole('button', { name: '查看琼华详情' }).click()
  await expect(page.locator('.n-drawer')).toHaveCSS('width', '390px')
  await page.screenshot({ path: path.join(artifacts, 'plugins-mobile-detail.png'), fullPage: true, animations: 'disabled' })
})

test('a deferred update remains pending and sends once, retaining saved secrets', async ({ page }) => {
  const state = await openPlugins(page, { deferred: true })
  await openConfig(page)
  await page.getByPlaceholder('已设置，留空保留原值').fill('')
  const address = page.locator('.n-drawer .n-form-item').filter({ has: page.locator('.n-form-item-label', { hasText: /^API 地址$/ }) }).locator('input')
  await address.fill('https://example.com/v1')
  await page.getByRole('button', { name: '保存并应用' }).click()
  await expect(page.getByRole('button', { name: '保存并应用' })).toBeDisabled()
  await expect(page.locator('.n-drawer')).toContainText('当前任务会继续完成')
  expect(state.writes).toHaveLength(1)
  expect(state.writes[0].settings.config).not.toHaveProperty('api_key')
  await expect(page.locator('.config-footer')).not.toContainText('配置已保存并生效')
  state.released = true
  await expect(page.locator('.config-footer')).toContainText('配置已保存并生效', { timeout: 8000 })
  expect(state.writes).toHaveLength(1)
})

test('background updates preserve drafts and failed saves explain the problem', async ({ page }) => {
  const state = await openPlugins(page, { error: true })
  await openConfig(page)
  const secret = page.getByPlaceholder('已设置，留空保留原值')
  await secret.fill('unsaved-test-only')
  const reads = state.reads
  await expect.poll(() => state.reads, { timeout: 7000 }).toBeGreaterThan(reads)
  await expect(secret).toHaveValue('unsaved-test-only')
  await page.getByRole('button', { name: '保存并应用' }).click()
  await expect(page.locator('.n-drawer')).toContainText('API 地址必须是有效的 HTTP(S) 链接')
  await expect(secret).toHaveValue('unsaved-test-only')
  await expect(page.getByRole('button', { name: '保存并应用' })).toBeEnabled()
})
