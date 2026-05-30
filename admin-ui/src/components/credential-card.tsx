import { useState } from 'react'
import { toast } from 'sonner'
import { Wallet } from 'lucide-react'
import { Card, CardContent, CardFooter, CardHeader } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from '@/components/ui/tooltip'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { CredentialStatusItem, CachedBalanceInfo, BalanceResponse } from '@/types/api'
import { cn } from '@/lib/utils'
import { BalanceBar } from '@/components/balance-bar'
import { CardActionsMenu } from '@/components/card-actions-menu'
import { CredentialEditPopover } from '@/components/credential-edit-popover'
import {
  useSetDisabled,
  useSetPriority,
  useSetRegion,
  useSetEndpoint,
  useResetFailure,
  useForceRefreshToken,
  useDeleteCredential,
} from '@/hooks/use-credentials'

interface CredentialCardProps {
  credential: CredentialStatusItem
  cachedBalance?: CachedBalanceInfo
  onViewBalance: (id: number, forceRefresh: boolean) => void
  selected: boolean
  onToggleSelect: () => void
  balance: BalanceResponse | null
  loadingBalance: boolean
}

function formatLastUsed(lastUsedAt: string | null): string {
  if (!lastUsedAt) return '从未使用'
  const date = new Date(lastUsedAt)
  const now = new Date()
  const diff = now.getTime() - date.getTime()
  if (diff < 0) return '刚刚'
  const seconds = Math.floor(diff / 1000)
  if (seconds < 60) return `${seconds} 秒前`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  const days = Math.floor(hours / 24)
  return `${days} 天前`
}

export function getOverspentCredits(balance: { remaining: number; usageLimit: number; currentUsage?: number }): number {
  const usageOverspend = balance.currentUsage !== undefined && balance.usageLimit > 0
    ? balance.currentUsage - balance.usageLimit
    : 0
  return Math.max(usageOverspend, -balance.remaining, 0)
}

const AUTH_METHOD_BADGE: Record<string, { label: string; className: string }> = {
  social: { label: 'Social', className: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400' },
  idc: { label: 'IdC', className: 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400' },
  api_key: { label: 'API Key', className: 'bg-orange-100 text-orange-700 dark:bg-orange-900/30 dark:text-orange-400' },
}

export function CredentialCard({
  credential,
  cachedBalance,
  onViewBalance,
  selected,
  onToggleSelect,
  balance,
  loadingBalance,
}: CredentialCardProps) {
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)
  const [editField, setEditField] = useState<'priority' | 'region' | 'endpoint' | null>(null)

  const setDisabled = useSetDisabled()
  const setPriority = useSetPriority()
  const setRegion = useSetRegion()
  const setEndpoint = useSetEndpoint()
  const resetFailure = useResetFailure()
  const forceRefreshToken = useForceRefreshToken()
  const deleteCredential = useDeleteCredential()

  const handleToggleDisabled = () => {
    setDisabled.mutate(
      { id: credential.id, disabled: !credential.disabled },
      {
        onSuccess: (res) => toast.success(res.message),
        onError: (err) => toast.error('操作失败: ' + (err as Error).message),
      }
    )
  }

  const handleReset = () => {
    resetFailure.mutate(credential.id, {
      onSuccess: (res) => toast.success(res.message),
      onError: (err) => toast.error('操作失败: ' + (err as Error).message),
    })
  }

  const handleForceRefresh = () => {
    forceRefreshToken.mutate(credential.id, {
      onSuccess: (res) => toast.success(res.message),
      onError: (err) => toast.error('刷新失败: ' + (err as Error).message),
    })
  }

  const handleDelete = () => {
    if (!credential.disabled) {
      toast.error('请先禁用凭据再删除')
      setShowDeleteDialog(false)
      return
    }
    deleteCredential.mutate(credential.id, {
      onSuccess: (res) => {
        toast.success(res.message)
        setShowDeleteDialog(false)
      },
      onError: (err) => toast.error('删除失败: ' + (err as Error).message),
    })
  }

  const handleEditSave = (field: string, value: string | number | Record<string, string | null>) => {
    setEditField(null)
    switch (field) {
      case 'priority':
        setPriority.mutate(
          { id: credential.id, priority: value as number },
          {
            onSuccess: (res) => toast.success(res.message),
            onError: (err) => toast.error('操作失败: ' + (err as Error).message),
          }
        )
        break
      case 'region': {
        const regionVal = value as Record<string, string | null>
        setRegion.mutate(
          { id: credential.id, region: regionVal.region, apiRegion: regionVal.apiRegion },
          {
            onSuccess: (res) => toast.success(res.message),
            onError: (err) => toast.error('操作失败: ' + (err as Error).message),
          }
        )
        break
      }
      case 'endpoint':
        setEndpoint.mutate(
          { id: credential.id, endpoint: (value as string) || null },
          {
            onSuccess: (res) => toast.success(res.message),
            onError: (err) => toast.error('操作失败: ' + (err as Error).message),
          }
        )
        break
    }
  }

  const isCacheStale = () => {
    if (!cachedBalance) return true
    const ageMs = Date.now() - cachedBalance.cachedAt
    const ttlMs = (cachedBalance.ttlSecs ?? 60) * 1000
    return ageMs > ttlMs
  }

  const handleViewBalance = () => onViewBalance(credential.id, isCacheStale())

  const displayEmail = credential.email || credential.accountEmail || `凭据 #${credential.id}`
  const authBadge = credential.authMethod ? AUTH_METHOD_BADGE[credential.authMethod] : null
  const totalFailures = credential.failureCount + credential.refreshFailureCount

  // Cached balance API may clamp remaining to 0 when overspent.
  // Calculate actual remaining from usagePercentage when overspent.
  const barRemaining = balance?.remaining
    ?? (cachedBalance && cachedBalance.usagePercentage > 100 && cachedBalance.usageLimit > 0
        ? cachedBalance.usageLimit - (cachedBalance.usagePercentage / 100) * cachedBalance.usageLimit
        : cachedBalance?.remaining)
    ?? null
  const barUsageLimit = balance?.usageLimit ?? cachedBalance?.usageLimit ?? null
  const barUsagePercentage = balance?.usagePercentage ?? cachedBalance?.usagePercentage ?? null
  const barSubscription = balance?.subscriptionTitle ?? cachedBalance?.subscriptionTitle ?? credential.subscriptionTitle ?? null

  const formatCacheAge = (cachedAt: number) => {
    const diff = Date.now() - cachedAt
    const seconds = Math.floor(diff / 1000)
    if (seconds < 60) return `${seconds}秒前`
    const minutes = Math.floor(seconds / 60)
    if (minutes < 60) return `${minutes}分钟前`
    return `${Math.floor(minutes / 60)}小时前`
  }

  return (
    <>
      <Card className={cn(credential.disabled && 'opacity-60')}>
        <CardHeader className="pb-2">
          <div className="flex items-center gap-2">
            <Checkbox
              checked={selected}
              onCheckedChange={onToggleSelect}
              aria-label={`选择凭据 ${displayEmail}`}
            />
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="truncate text-sm font-medium min-w-0 flex-1">
                  {displayEmail}
                </span>
              </TooltipTrigger>
              <TooltipContent>{displayEmail}</TooltipContent>
            </Tooltip>
            {authBadge && (
              <Badge variant="outline" className={cn('text-xs shrink-0', authBadge.className)}>
                {authBadge.label}
              </Badge>
            )}
            <Tooltip>
              <TooltipTrigger asChild>
                <span>
                  <Badge
                    variant={credential.disabled ? 'secondary' : 'default'}
                    className={cn(
                      'text-xs shrink-0',
                      !credential.disabled && 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                    )}
                  >
                    {credential.disabled ? '已禁用' : '启用'}
                  </Badge>
                </span>
              </TooltipTrigger>
              {credential.disabled && credential.disabledReason && (
                <TooltipContent>{credential.disabledReason}</TooltipContent>
              )}
            </Tooltip>
            <Switch
              checked={!credential.disabled}
              onCheckedChange={handleToggleDisabled}
              disabled={setDisabled.isPending}
              className="shrink-0 ml-auto"
              aria-label={`${displayEmail} 启用状态`}
            />
          </div>
        </CardHeader>

        <CardContent className="space-y-3 pb-3">
          <button
            type="button"
            className="w-full text-left rounded-md p-2 -mx-2 hover:bg-muted/50 transition-colors"
            onClick={handleViewBalance}
            aria-label="查看余额详情"
          >
            <BalanceBar
              remaining={barRemaining}
              usageLimit={barUsageLimit}
              usagePercentage={barUsagePercentage}
              subscriptionTitle={barSubscription ?? undefined}
              cachedAt={cachedBalance ? formatCacheAge(cachedBalance.cachedAt) : undefined}
            />
          </button>

          <div className="flex flex-wrap items-center gap-1.5 text-xs">
            {barSubscription && (
              <Badge variant="outline" className="text-xs font-normal">
                {barSubscription}
              </Badge>
            )}
            {totalFailures > 0 && (
              <Badge variant="destructive" className="text-xs">
                {totalFailures} 次失败
              </Badge>
            )}
            <span className="text-muted-foreground">
              {formatLastUsed(credential.lastUsedAt)}
            </span>
            {cachedBalance && !balance && (
              <span className="text-muted-foreground">
                · 缓存 {formatCacheAge(cachedBalance.cachedAt)}
              </span>
            )}
          </div>
        </CardContent>

        <CardFooter className="pt-0 pb-3 flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={handleViewBalance} disabled={loadingBalance}>
            <Wallet className="h-3.5 w-3.5 mr-1" aria-hidden="true" />
            查看余额
          </Button>
          <div className="ml-auto flex items-center gap-1">
            {editField && (
              <CredentialEditPopover
                key={editField}
                field={editField}
                credential={credential}
                onSave={handleEditSave}
                isPending={setPriority.isPending || setRegion.isPending || setEndpoint.isPending}
                defaultOpen
                onClose={() => setEditField(null)}
                trigger={<span className="sr-only" />}
              />
            )}
            <CardActionsMenu
              credential={credential}
              onResetFailures={handleReset}
              onRefreshToken={handleForceRefresh}
              onViewBalance={handleViewBalance}
              onDelete={() => setShowDeleteDialog(true)}
              onEditPriority={() => setEditField('priority')}
              onEditRegion={() => setEditField('region')}
              onEditEndpoint={() => setEditField('endpoint')}
              isResetting={resetFailure.isPending}
              isRefreshing={forceRefreshToken.isPending}
              isDeleting={deleteCredential.isPending}
            />
          </div>
        </CardFooter>
      </Card>

      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认删除凭据</DialogTitle>
            <DialogDescription>
              您确定要删除凭据 #{credential.id} 吗？此操作无法撤销。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowDeleteDialog(false)}
              disabled={deleteCredential.isPending}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              onClick={handleDelete}
              disabled={deleteCredential.isPending || !credential.disabled}
            >
              确认删除
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
