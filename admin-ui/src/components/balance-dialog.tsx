import { useEffect } from 'react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Progress } from '@/components/ui/progress'
import { Switch } from '@/components/ui/switch'
import { Loader2 } from 'lucide-react'
import { toast } from 'sonner'
import { useCredentialBalance, useSetOverage } from '@/hooks/use-credentials'
import { formatKiroCredits, formatKiroCreditsAsUsd } from '@/lib/format'
import { cn, parseError } from '@/lib/utils'
import type { BalanceResponse } from '@/types/api'

interface BalanceDialogProps {
  credentialId: number | null
  open: boolean
  onOpenChange: (open: boolean) => void
  forceRefresh?: boolean
  onBalanceLoaded?: (credentialId: number, balance: BalanceResponse) => void
}

function isPaidTier(subscriptionTitle: string | null): boolean {
  const normalized = subscriptionTitle?.trim().toUpperCase() ?? ''
  return normalized !== '' && !normalized.includes('FREE')
}

export function BalanceDialog({ credentialId, open, onOpenChange, forceRefresh, onBalanceLoaded }: BalanceDialogProps) {
  const { data: balance, isLoading, isFetching, error } = useCredentialBalance(credentialId)
  const setOverage = useSetOverage()
  const showLoading = isLoading || (forceRefresh && isFetching)

  useEffect(() => {
    if (credentialId !== null && balance) {
      onBalanceLoaded?.(credentialId, balance)
    }
  }, [balance, credentialId, onBalanceLoaded])

  const formatDate = (timestamp: number | null) => {
    if (!timestamp) return '未知'
    return new Date(timestamp * 1000).toLocaleString('zh-CN')
  }

  const handleOverageChange = (enabled: boolean) => {
    if (credentialId === null) return
    setOverage.mutate(
      { id: credentialId, overageEnabled: enabled },
      {
        onSuccess: (res) => toast.success(res.message),
        onError: (err) => {
          const parsed = parseError(err)
          toast.error(parsed.detail ? `${parsed.title}: ${parsed.detail}` : parsed.title)
        },
      }
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            凭据 #{credentialId} 余额信息
          </DialogTitle>
          <DialogDescription>
            以 Kiro credits 为主，美元金额按 1 credit = $0.04 估算。
          </DialogDescription>
        </DialogHeader>

        {showLoading && (
          <div className="flex items-center justify-center py-8" aria-live="polite" aria-busy="true">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary" aria-hidden="true"></div>
          </div>
        )}

        {error && (() => {
          const parsed = parseError(error)
          return (
            <div className="py-6 space-y-3">
              <div className="flex items-center justify-center gap-2 text-red-500">
                <svg className="h-5 w-5" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
                  <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clipRule="evenodd" />
                </svg>
                <span className="font-medium">{parsed.title}</span>
              </div>
              {parsed.detail && (
                <div className="text-sm text-muted-foreground text-center px-4">
                  {parsed.detail}
                </div>
              )}
            </div>
          )
        })()}

        {balance && (() => {
          const overspentCredits = Math.max(balance.currentUsage - balance.usageLimit, -balance.remaining, 0)
          const isOverspent = overspentCredits > 0
          const displayedRemaining = isOverspent ? -overspentCredits : balance.remaining
          const canOperateOverage = isPaidTier(balance.subscriptionTitle) && balance.overageEnabled !== null && balance.overageEnabled !== undefined
          const actualUsagePercentage = balance.usageLimit > 0
            ? (balance.currentUsage / balance.usageLimit) * 100
            : balance.usagePercentage

          return (
            <div className="space-y-4">
            {/* 订阅类型 */}
            <div className="text-center">
              <span className="text-lg font-semibold">
                {balance.subscriptionTitle || '未知订阅类型'}
              </span>
            </div>

            {/* 使用进度 */}
            <div className="space-y-2">
              <div className="grid grid-cols-2 gap-3 text-sm">
                <div>
                  <div className="text-muted-foreground">已使用</div>
                  <div className="font-medium tabular-nums">{formatKiroCredits(balance.currentUsage)}</div>
                  <div className="text-xs text-muted-foreground tabular-nums">≈ {formatKiroCreditsAsUsd(balance.currentUsage)}</div>
                </div>
                <div className="text-right">
                  <div className="text-muted-foreground">限额</div>
                  <div className="font-medium tabular-nums">{formatKiroCredits(balance.usageLimit)}</div>
                  <div className="text-xs text-muted-foreground tabular-nums">≈ {formatKiroCreditsAsUsd(balance.usageLimit)}</div>
                </div>
              </div>
              <Progress value={actualUsagePercentage} />
              <div className={cn('text-center text-sm', isOverspent ? 'font-medium text-destructive' : 'text-muted-foreground')}>
                {actualUsagePercentage.toFixed(1)}% 已使用
              </div>
            </div>

            {/* 详细信息 */}
            <div className="grid grid-cols-2 gap-4 pt-4 border-t text-sm">
              <div>
                <span className="text-muted-foreground">{isOverspent ? '已超支：' : '剩余额度：'}</span>
                <div className={cn('font-medium tabular-nums', isOverspent ? 'text-destructive' : 'text-green-600')}>
                  {formatKiroCredits(displayedRemaining)}
                </div>
                <div className="text-xs text-muted-foreground tabular-nums">
                  ≈ {formatKiroCreditsAsUsd(displayedRemaining)}
                </div>
              </div>
              <div>
                <span className="text-muted-foreground">下次重置：</span>
                <span className="font-medium">
                  {formatDate(balance.nextResetAt)}
                </span>
              </div>
            </div>

            {canOperateOverage && (
              <div className="rounded-lg border bg-muted/40 p-3 space-y-3">
                <div className="flex items-start justify-between gap-3">
                  <div className="space-y-1">
                    <div className="text-sm font-medium">Overage</div>
                    <div className="text-xs text-muted-foreground">
                      允许该付费凭据在基础 credits 用完后继续按量使用。
                    </div>
                  </div>
                  <Switch
                    checked={Boolean(balance.overageEnabled)}
                    onCheckedChange={handleOverageChange}
                    disabled={setOverage.isPending}
                    aria-label={`凭据 ${credentialId} overage 开关`}
                  />
                </div>
                <div className="flex items-center justify-between text-xs">
                  <span className="text-muted-foreground">当前状态</span>
                  <span className={cn(
                    'font-medium tabular-nums',
                    balance.overageEnabled ? 'text-green-600' : 'text-muted-foreground'
                  )}>
                    {balance.overageStatus ?? (balance.overageEnabled ? 'ENABLED' : 'DISABLED')}
                  </span>
                </div>
                {setOverage.isPending && (
                  <div className="flex items-center gap-2 text-xs text-muted-foreground" aria-live="polite">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
                    正在提交并确认 upstream 状态...
                  </div>
                )}
                {isOverspent && !balance.overageEnabled && (
                  <div className="text-xs text-destructive">
                    当前已用完基础 credits；开启失败通常表示 upstream 未授权该 profile 使用 overage。
                  </div>
                )}
              </div>
            )}

            {!canOperateOverage && isPaidTier(balance.subscriptionTitle) && (
              <div className="rounded-lg border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground">
                此凭据没有返回可操作的 overage 状态；请先刷新余额后再尝试。
              </div>
            )}

            {setOverage.error && (() => {
              const parsed = parseError(setOverage.error)
              return (
                <div className="rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-xs text-destructive" role="alert">
                  <div className="font-medium">{parsed.title}</div>
                  {parsed.detail && <div className="mt-1">{parsed.detail}</div>}
                </div>
              )
            })()}
            </div>
          )
        })()}
      </DialogContent>
    </Dialog>
  )
}
