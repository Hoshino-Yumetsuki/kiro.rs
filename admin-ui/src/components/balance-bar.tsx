import { Progress } from '@/components/ui/progress'
import { cn } from '@/lib/utils'
import { formatKiroCreditAmount } from '@/lib/format'

interface BalanceBarProps {
  remaining: number | null
  usageLimit: number | null
  usagePercentage: number | null
  subscriptionTitle?: string
  cachedAt?: string
}

export function BalanceBar({
  remaining,
  usageLimit,
  usagePercentage,
  subscriptionTitle,
}: BalanceBarProps) {
  if (usageLimit === null || usageLimit === 0) {
    return (
      <div className="space-y-1">
        <div className="flex items-center justify-between text-xs">
          <span className="text-muted-foreground">无限额</span>
          {subscriptionTitle && (
            <span className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground">
              {subscriptionTitle}
            </span>
          )}
        </div>
      </div>
    )
  }

  if (remaining === null) {
    return (
      <div className="space-y-1">
        <Progress value={0} />
        <div className="flex items-center justify-between text-xs">
          <span className="text-muted-foreground">无数据</span>
          {subscriptionTitle && (
            <span className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground">
              {subscriptionTitle}
            </span>
          )}
        </div>
      </div>
    )
  }

  const isOverspent = remaining < 0

  return (
    <div className="space-y-1">
      <Progress
        value={isOverspent ? 100 : (usagePercentage ?? 0)}
        className={cn(isOverspent && '[&>div]:!bg-destructive')}
      />
      <div className="flex items-center justify-between text-xs">
        <span
          className={cn(
            'tabular-nums',
            isOverspent ? 'font-medium text-destructive' : 'text-muted-foreground'
          )}
        >
          {formatKiroCreditAmount(remaining)} / {formatKiroCreditAmount(usageLimit)} credits
          {isOverspent && ` (${(usagePercentage ?? 0).toFixed(1)}%)`}
        </span>
        {subscriptionTitle && (
          <span className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground">
            {subscriptionTitle}
          </span>
        )}
      </div>
    </div>
  )
}
