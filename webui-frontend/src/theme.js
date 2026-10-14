// 全局主题覆盖：现代无衬线字体 + 等宽数字/代码字体 + 清爽现代锌灰与钴蓝配色
const FONT = "'Inter', -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'PingFang SC', 'HarmonyOS Sans SC', 'Noto Sans SC', system-ui, sans-serif"
const MONO = "'JetBrains Mono', 'SFMono-Regular', 'Cascadia Code', 'Fira Code', Menlo, Consolas, monospace"

const base = {
  common: {
    fontFamily: FONT,
    fontFamilyMono: MONO,
    fontSize: '14px',
    borderRadius: '8px',
    borderRadiusSmall: '6px',
  },
  Card: {
    borderRadius: '12px',
    titleFontSizeSmall: '14px',
    titleFontSizeMedium: '15px',
    titleFontWeight: '600',
    paddingSmall: '16px 20px',
  },
  Menu: {
    borderRadius: '8px',
    itemHeight: '36px',
  },
  Tag: {
    borderRadius: '6px',
    fontWeightStrong: '500',
  },
  Button: {
    borderRadiusMedium: '8px',
    borderRadiusSmall: '7px',
    borderRadiusTiny: '6px',
    fontWeight: '500',
  },
  Input: {
    borderRadius: '8px',
  },
  DataTable: {
    borderRadius: '10px',
  },
}

function palette(dark) {
  const primary = dark ? '#60a5fa' : '#2563eb'
  const primaryHover = dark ? '#93c5fd' : '#1d4ed8'
  const primaryPressed = dark ? '#3b82f6' : '#1e40af'
  const text = dark ? '#f8fafc' : '#0f172a'
  const text2 = dark ? '#cbd5e1' : '#334155'
  const muted = dark ? '#64748b' : '#64748b'
  const body = dark ? '#0b0f17' : '#f8fafc'
  const surface = dark ? '#111827' : '#ffffff'
  const elevated = dark ? '#1e293b' : '#f1f5f9'
  const border = dark ? '#1e293b' : '#e2e8f0'
  const active = dark ? 'rgba(96, 165, 250, 0.14)' : '#eff6ff'

  const colors = {
    primary,
    info: primary,
    success: dark ? '#34d399' : '#10b981',
    warning: dark ? '#fbbf24' : '#f59e0b',
    error: dark ? '#f87171' : '#ef4444',
  }

  const common = {
    ...base.common,
    bodyColor: body,
    cardColor: surface,
    modalColor: surface,
    popoverColor: surface,
    tableColor: surface,
    inputColor: dark ? '#0f172a' : '#ffffff',
    textColorBase: text,
    textColor1: text,
    textColor2: text2,
    textColor3: muted,
    borderColor: border,
    dividerColor: border,
    primaryColorHover: primaryHover,
    primaryColorPressed: primaryPressed,
  }

  for (const [name, color] of Object.entries(colors)) {
    common[`${name}Color`] = color
    if (name !== 'primary') {
      common[`${name}ColorHover`] = color
      common[`${name}ColorPressed`] = color
    }
    common[`${name}ColorSuppl`] = color
  }

  return {
    ...base,
    common,
    Layout: {
      color: body,
      headerColor: surface,
      siderColor: surface,
    },
    Card: {
      ...base.Card,
      color: surface,
      borderColor: border,
      textColor: text,
      titleTextColor: text,
    },
    DataTable: {
      ...base.DataTable,
      tdColor: surface,
      tdColorHover: elevated,
      thColor: elevated,
      thTextColor: text2,
      borderColor: border,
    },
    Menu: {
      ...base.Menu,
      itemColorActive: active,
      itemColorActiveHover: active,
      itemTextColorActive: primary,
      itemIconColorActive: primary,
      groupTextColor: muted,
    },
  }
}

export const lightOverrides = palette(false)
export const darkOverrides = palette(true)
