import { expect, test } from '@playwright/test'
import { readFileSync } from 'node:fs'

const defaults = {
  content: readFileSync(new URL('../../crates/core/src/personality/default.txt', import.meta.url), 'utf8'),
  reply_examples: JSON.parse(readFileSync(new URL('../../crates/core/src/personality/examples.json', import.meta.url), 'utf8')),
}

async function openPersonality(page, { saveError = false } = {}) {
  const versions = [{ version_no: 1, content: '旧版性格', reply_examples: [], active: true, note: '旧版', created_at: 1 }]
  const writes = []
  await page.addInitScript(() => localStorage.setItem('yt_token', 'test-only'))
  await page.routeWebSocket('**/ws**', () => {})
  await page.route('**/api/**', async route => {
    const request = route.request()
    const path = new URL(request.url()).pathname
    if (path === '/api/personality/defaults') return route.fulfill({ json: defaults })
    if (path === '/api/personality/versions' && request.method() === 'GET') {
      return route.fulfill({ json: { versions: [...versions].reverse().map(v => ({ ...v, content_len: v.content.length, reply_examples_count: v.reply_examples.length })) } })
    }
    if (path === '/api/personality/versions' && request.method() === 'POST') {
      writes.push(request.postDataJSON())
      if (saveError) return route.fulfill({ status: 400, json: { error: '模拟保存失败' } })
      versions.forEach(v => { v.active = false })
      const saved = { ...structuredClone(request.postDataJSON()), version_no: versions.length + 1, active: true, created_at: 2 }
      versions.push(saved)
      return route.fulfill({ json: { ok: true, version_no: saved.version_no } })
    }
    if (path.startsWith('/api/personality/versions/')) {
      return route.fulfill({ json: versions.find(v => v.version_no === Number(path.split('/').at(-1))) })
    }
    if (path.startsWith('/api/personality/rollback/')) {
      const source = versions.find(v => v.version_no === Number(path.split('/').at(-1)))
      versions.forEach(v => { v.active = false })
      const saved = { ...structuredClone(source), version_no: versions.length + 1, active: true, note: `回滚自 v${source.version_no}` }
      versions.push(saved)
      return route.fulfill({ json: { ok: true, version_no: saved.version_no } })
    }
    return route.fulfill({ json: {} })
  })
  await page.goto('/personality')
  await expect(page.getByPlaceholder('输入人物性格系统提示词')).toHaveValue('旧版性格')
  return { versions, writes }
}

test('template and edited Q&A survive saving and reload; rollback restores both fields', async ({ page }) => {
  const { versions } = await openPersonality(page)
  await page.getByRole('button', { name: '载入云团模板' }).click()
  await expect(page.locator('.example-pair')).toHaveCount(12)
  await expect(page.getByPlaceholder('输入人物性格系统提示词')).toHaveValue(defaults.content)
  await page.getByRole('button', { name: '删除示例 12', exact: true }).click()
  await page.getByRole('button', { name: '添加一问一答' }).click()
  await page.getByLabel('用户提问', { exact: true }).last().fill('测试保存问题')
  await page.getByLabel('云团回复', { exact: true }).last().fill('简洁且俏皮的回答。')
  await page.getByRole('button', { name: '保存并激活新版本' }).click()
  await expect(page.getByText('已保存 v2 并设为当前生效版本')).toBeVisible()
  expect(versions[1].reply_examples[11]).toEqual({ question: '测试保存问题', answer: '简洁且俏皮的回答。' })
  await page.reload()
  await expect(page.getByLabel('云团回复', { exact: true }).last()).toHaveValue('简洁且俏皮的回答。')
  const old = page.locator('.version-row').filter({ has: page.getByText('v1', { exact: true }) })
  await old.getByRole('button', { name: '回滚', exact: true }).click()
  await page.getByRole('button', { name: '确认', exact: true }).click()
  await expect(page.getByPlaceholder('输入人物性格系统提示词')).toHaveValue('旧版性格')
  await expect(page.locator('.example-pair')).toHaveCount(0)
  await expect(page.locator('.version-row.active')).toContainText('v3')
})

test('incomplete pair is blocked and failed save keeps draft', async ({ page }) => {
  const { writes } = await openPersonality(page, { saveError: true })
  await page.getByRole('button', { name: '添加一问一答' }).click()
  await page.getByLabel('用户提问', { exact: true }).fill('还未填写回答')
  await page.getByRole('button', { name: '保存并激活新版本' }).click()
  await expect(page.getByText('第 1 组示例的问题和回答都不能为空')).toBeVisible()
  expect(writes).toHaveLength(0)
  await page.getByLabel('云团回复', { exact: true }).fill('不要丢失这条回答')
  await page.getByRole('button', { name: '保存并激活新版本' }).click()
  await expect(page.getByText('模拟保存失败')).toBeVisible()
  await expect(page.getByLabel('云团回复', { exact: true })).toHaveValue('不要丢失这条回答')
  expect(writes).toHaveLength(1)
})

test('versions compare question and answer changes even when prompt is unchanged', async ({ page }) => {
  await openPersonality(page)
  await page.getByRole('button', { name: '添加一问一答' }).click()
  await page.getByLabel('用户提问', { exact: true }).fill('示例对比问题')
  await page.getByLabel('云团回复', { exact: true }).fill('示例对比回答')
  await page.getByRole('button', { name: '保存并激活新版本' }).click()
  await expect(page.locator('.version-row')).toHaveCount(2)
  await page.getByRole('checkbox').nth(0).check()
  await page.getByRole('checkbox').nth(1).check()
  await expect(page.locator('.diff-box')).toContainText('示例对比问题')
  await expect(page.locator('.diff-box')).toContainText('示例对比回答')
})
