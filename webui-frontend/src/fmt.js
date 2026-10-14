export function fmtTs(ts) {
  return ts ? new Date(ts * 1000).toLocaleString('zh-CN', { hour12: false }) : '—'
}

export function fmtAgo(ts) {
  if (!ts) return '—'
  const s = Math.max(0, Math.floor(Date.now() / 1000 - ts))
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  return `${Math.floor(h / 24)}d`
}

export function fmtUptime(s) {
  if (s == null) return '—'
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  return h > 0 ? `${h}h ${m}m` : `${m}m ${s % 60}s`
}

export const KIND_COLORS = {
  MessageReceived: 'var(--yt-ink-3)',
  DecisionMade: 'var(--yt-primary)',
  BubbleSent: 'var(--yt-ok)',
  ReplyInterrupted: 'var(--yt-danger)',
  ConsolidationDone: 'var(--yt-primary)',
  MemoryWritten: 'var(--yt-ink-2)',
  MoodChanged: 'var(--yt-warning)',
  ConfigReloaded: 'var(--yt-ink-2)',
  TaskCreated: 'var(--yt-primary)',
  TaskFinished: 'var(--yt-ok)',
}

export function kindColor(kind) {
  return KIND_COLORS[kind] || 'var(--yt-ink-3)'
}

export const ACTION_COLORS = {
  reply: 'var(--yt-ok)',
  ignore: 'var(--yt-ink-3)',
  send_meme: 'var(--yt-primary)',
  start_task: 'var(--yt-warning)',
}

export function actionColor(action) {
  return ACTION_COLORS[action] || 'var(--yt-ink-3)'
}

export const KIND_LABELS = {
  MessageReceived: '收到消息',
  DecisionMade: '策略决策',
  BubbleSent: '发送回复',
  ReplyInterrupted: '回复中断',
  ConsolidationDone: '记忆归纳',
  MemoryWritten: '写入记忆',
  MoodChanged: '情绪变更',
  ConfigReloaded: '配置重载',
  TaskCreated: '创建任务',
  TaskFinished: '任务完成',
}

export function kindLabel(kind) {
  return KIND_LABELS[kind] || kind
}

export function eventSummary(e) {
  const p = e.payload || {}
  if (e.kind === 'DecisionMade') {
    const parts = [p.action, p.mood, p.reason].filter(Boolean)
    return parts.join(' · ')
  }
  if (p.text) return String(p.text)
  if (p.chat_id) return String(p.chat_id)
  return e.kind
}
