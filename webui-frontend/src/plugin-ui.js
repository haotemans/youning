export function pluginStatus(plugin, pending = false) {
  if (pending || plugin.updating) return { key: 'applying', label: '正在应用', tone: 'working' }
  if (plugin.pending_restart) return { key: 'restart', label: '需要重启服务', tone: 'error' }
  if (plugin.pending_reload || plugin.error) return { key: 'attention', label: '需要处理', tone: 'warning' }
  if (!plugin.enabled) return { key: 'disabled', label: '未启用', tone: 'muted' }
  if (!plugin.loaded) return { key: 'unavailable', label: '未加载', tone: 'warning' }
  if (plugin.invocations?.active) return { key: 'running', label: `${plugin.invocations.active} 个任务执行中`, tone: 'working' }
  return { key: 'ready', label: '已启用', tone: 'success' }
}

export function pluginDuration(milliseconds) {
  if (milliseconds == null) return '—'
  if (milliseconds < 1000) return `${milliseconds} ms`
  return `${(milliseconds / 1000).toFixed(1)} s`
}
