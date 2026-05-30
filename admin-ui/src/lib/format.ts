export function formatCompactNumber(value: number | null | undefined): string {
  if (value === null || value === undefined) return '-'
  if (!Number.isFinite(value)) return String(value)

  const abs = Math.abs(value)
  const sign = value < 0 ? '-' : ''

  const formatScaled = (scale: number, suffix: string) => {
    const scaled = abs / scale
    const decimals = scaled < 10 ? 1 : 0
    const fixed = scaled.toFixed(decimals)
    const trimmed = fixed.endsWith('.0') ? fixed.slice(0, -2) : fixed
    return `${sign}${trimmed}${suffix}`
  }

  if (abs >= 1_000_000_000) return formatScaled(1_000_000_000, 'B')
  if (abs >= 1_000_000) return formatScaled(1_000_000, 'M')
  if (abs >= 1_000) return formatScaled(1_000, 'K')

  // 小于 1000：按整数显示
  return `${sign}${Math.round(abs)}`
}

export function formatTokensPair(inputTokens: number, outputTokens: number): string {
  return `${formatCompactNumber(inputTokens)} in / ${formatCompactNumber(outputTokens)} out`
}

const KIRO_CREDIT_USD_RATE = 0.04

export function formatKiroCreditAmount(credits: number): string {
  return `${credits.toLocaleString('zh-CN', {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`
}

export function formatKiroCredits(credits: number): string {
  return `${formatKiroCreditAmount(credits)} credits`
}

export function formatKiroCreditsAsUsd(credits: number): string {
  return new Intl.NumberFormat('zh-CN', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(credits * KIRO_CREDIT_USD_RATE)
}

export function formatKiroCreditsWithUsd(credits: number): string {
  return `${formatKiroCredits(credits)} (≈ ${formatKiroCreditsAsUsd(credits)})`
}

export function formatKiroUsageWithUsd(currentUsage: number, usageLimit: number): string {
  return `${formatKiroCreditAmount(currentUsage)} / ${formatKiroCredits(usageLimit)} (≈ ${formatKiroCreditsAsUsd(currentUsage)} / ${formatKiroCreditsAsUsd(usageLimit)})`
}

export function formatExpiry(expiresAt: string | null): string {
  if (!expiresAt) return '未知'
  const date = new Date(expiresAt)
  if (isNaN(date.getTime())) return expiresAt

  const now = new Date()
  const diff = date.getTime() - now.getTime()
  if (diff < 0) return '已过期'

  const minutes = Math.floor(diff / 60000)
  if (minutes < 60) return `${minutes} 分钟`

  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时`

  return `${Math.floor(hours / 24)} 天`
}
