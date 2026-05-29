import { useState } from 'react'
import { toast } from 'sonner'
import { RefreshCw, RotateCcw, Wallet, Trash2, Loader2, Pencil, Check, X } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { CredentialStatusItem, CachedBalanceInfo, BalanceResponse } from '@/types/api'
import { formatKiroCredits, formatKiroCreditsAsUsd } from '@/lib/format'
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

export function CredentialCard({
  credential,
  cachedBalance,
  onViewBalance,
  selected,
  onToggleSelect,
  balance,
  loadingBalance,
}: CredentialCardProps) {
  const [editingPriority, setEditingPriority] = useState(false)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [editingRegion, setEditingRegion] = useState(false)
  const [regionValue, setRegionValue] = useState(credential.region ?? '')
  const [apiRegionValue, setApiRegionValue] = useState(credential.apiRegion ?? '')
  const [editingEndpoint, setEditingEndpoint] = useState(false)
  const [endpointValue, setEndpointValue] = useState(credential.endpoint ?? '')
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)

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
        onSuccess: (res) => {
          toast.success(res.message)
        },
        onError: (err) => {
          toast.error('操作失败: ' + (err as Error).message)
        },
      }
    )
  }

  const handlePriorityChange = () => {
    const newPriority = parseInt(priorityValue, 10)
    if (isNaN(newPriority) || newPriority < 0) {
      toast.error('优先级必须是非负整数')
      return
    }
    setPriority.mutate(
      { id: credential.id, priority: newPriority },
      {
        onSuccess: (res) => {
          toast.success(res.message)
          setEditingPriority(false)
        },
        onError: (err) => {
          toast.error('操作失败: ' + (err as Error).message)
        },
      }
    )
  }

  const handleRegionChange = () => {
    setRegion.mutate(
      {
        id: credential.id,
        region: regionValue.trim() || null,
        apiRegion: apiRegionValue.trim() || null,
      },
      {
        onSuccess: (res) => {
          toast.success(res.message)
          setEditingRegion(false)
        },
        onError: (err) => {
          toast.error('操作失败: ' + (err as Error).message)
        },
      }
    )
  }

  const handleEndpointChange = () => {
    setEndpoint.mutate(
      {
        id: credential.id,
        endpoint: endpointValue || null,
      },
      {
        onSuccess: (res) => {
          toast.success(res.message)
          setEditingEndpoint(false)
        },
        onError: (err) => {
          toast.error('操作失败: ' + (err as Error).message)
        },
      }
    )
  }

  const handleReset = () => {
    resetFailure.mutate(credential.id, {
      onSuccess: (res) => {
        toast.success(res.message)
      },
      onError: (err) => {
        toast.error('操作失败: ' + (err as Error).message)
      },
    })
  }

  const handleForceRefresh = () => {
    forceRefreshToken.mutate(credential.id, {
      onSuccess: (res) => {
        toast.success(res.message)
      },
      onError: (err) => {
        toast.error('刷新失败: ' + (err as Error).message)
      },
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
      onError: (err) => {
        toast.error('删除失败: ' + (err as Error).message)
      },
    })
  }

  // 格式化缓存时间（相对时间）
  const formatCacheAge = (cachedAt: number) => {
    const now = Date.now()
    const diff = now - cachedAt
    const seconds = Math.floor(diff / 1000)
    if (seconds < 60) return `${seconds}秒前`
    const minutes = Math.floor(seconds / 60)
    if (minutes < 60) return `${minutes}分钟前`
    return `${Math.floor(minutes / 60)}小时前`
  }

  // 检查缓存是否过期（使用后端返回的 TTL）
  const isCacheStale = () => {
    if (!cachedBalance) return true
    const ageMs = Date.now() - cachedBalance.cachedAt
    const ttlMs = (cachedBalance.ttlSecs ?? 60) * 1000
    return ageMs > ttlMs
  }

  const handleViewBalance = () => {
    onViewBalance(credential.id, isCacheStale())
  }


  return (
    <>
      <Card>
        <CardHeader className="pb-2">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 min-w-0 flex-1">
              <Checkbox
                checked={selected}
                onCheckedChange={onToggleSelect}
                aria-label={`选择凭据 ${credential.email || `#${credential.id}`}`}
              />
              <CardTitle className="text-lg flex items-center gap-2 min-w-0 flex-1">
                <span className="truncate">{credential.email || `凭据 #${credential.id}`}</span>
                <div className="flex items-center gap-1 flex-shrink-0">
                  {credential.disabled && (
                    <Badge variant="destructive">已禁用</Badge>
                  )}
                  {credential.disabled && credential.disabledReason && (
                    <Badge variant="outline">{credential.disabledReason}</Badge>
                  )}
                  {credential.authMethod && (
                    <Badge variant="secondary">
                      {credential.authMethod === 'api_key' ? 'API Key' :
                       credential.authMethod === 'idc' ? 'IdC' :
                       credential.authMethod === 'social' ? 'Social' :
                       credential.authMethod}
                    </Badge>
                  )}
                </div>
              </CardTitle>
            </div>
            <div className="flex items-center gap-2 flex-shrink-0">
              <span className="text-sm text-muted-foreground">启用</span>
              <Switch
                checked={!credential.disabled}
                onCheckedChange={handleToggleDisabled}
                disabled={setDisabled.isPending}
                aria-label={`${credential.email || `凭据 #${credential.id}`} 启用状态`}
              />
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* 信息网格 */}
          <div className="grid grid-cols-2 gap-4 text-sm">
            <div>
              <span className="text-muted-foreground">优先级：</span>
              {editingPriority ? (
                <div className="inline-flex items-center gap-1 ml-1">
                  <Input
                    type="number"
                    value={priorityValue}
                    onChange={(e) => setPriorityValue(e.target.value)}
                    className="w-16 h-7 text-sm"
                    min="0"
                    aria-label="优先级"
                  />
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 w-7 p-0"
                    onClick={handlePriorityChange}
                    disabled={setPriority.isPending}
                    aria-label="确认"
                  >
                    <Check className="h-4 w-4" aria-hidden="true" />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 w-7 p-0"
                    onClick={() => {
                      setEditingPriority(false)
                      setPriorityValue(String(credential.priority))
                    }}
                    aria-label="取消"
                  >
                    <X className="h-4 w-4" aria-hidden="true" />
                  </Button>
                </div>
              ) : (
                <button
                  type="button"
                  className="group inline-flex items-center gap-1 font-medium ml-1 text-left bg-transparent border-0 p-0 hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring rounded-sm"
                  onClick={() => setEditingPriority(true)}
                  aria-label={`编辑优先级（当前 ${credential.priority}）`}
                >
                  {credential.priority}
                  <Pencil className="h-3 w-3 text-muted-foreground opacity-50 group-hover:opacity-100 transition-opacity" aria-hidden="true" />
                </button>
              )}
            </div>
            <div>
              <span className="text-muted-foreground">失败次数：</span>
              <span className={credential.failureCount > 0 ? 'text-red-500 font-medium' : ''}>
                {credential.failureCount}
              </span>
              {credential.refreshFailureCount > 0 && (
                <span className="ml-2 text-amber-600 font-medium">
                  刷新 {credential.refreshFailureCount}
                </span>
              )}
            </div>
            <div>
              <span className="text-muted-foreground">订阅等级：</span>
              <span className="font-medium">
                {loadingBalance ? (
                  <Loader2 className="inline w-3 h-3 animate-spin" aria-hidden="true" />
                ) : balance?.subscriptionTitle ?? cachedBalance?.subscriptionTitle ?? credential.subscriptionTitle ?? '未知'}
              </span>
            </div>
            <div>
              <span className="text-muted-foreground">成功次数：</span>
              <span className="font-medium">{credential.successCount}</span>
            </div>
            <div className="col-span-2">
              <span className="text-muted-foreground">最后调用：</span>
              <span className="font-medium">{formatLastUsed(credential.lastUsedAt)}</span>
            </div>
            {credential.disabledReason && (
              <div className="col-span-2">
                <span className="text-muted-foreground">禁用原因：</span>
                <span className="font-medium">{credential.disabledReason}</span>
              </div>
            )}
            {credential.maskedApiKey && (
              <div className="col-span-2">
                <span className="text-muted-foreground">API Key：</span>
                <span className="font-mono font-medium">{credential.maskedApiKey}</span>
              </div>
            )}
            <div className="col-span-2">
              <span className="text-muted-foreground">余额：</span>
              {loadingBalance ? (
                <span className="text-sm ml-1">
                  <Loader2 className="inline w-3 h-3 animate-spin" aria-hidden="true" /> 加载中…
                </span>
              ) : balance ? (
                <span className="inline-flex flex-col gap-0.5 ml-1 align-top">
                  <span className={`font-medium tabular-nums ${balance.remaining < 0 ? 'text-destructive' : 'text-green-600'}`}>
                    {formatKiroCredits(balance.remaining)} / {formatKiroCredits(balance.usageLimit)}
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    ≈ {formatKiroCreditsAsUsd(balance.remaining)} / {formatKiroCreditsAsUsd(balance.usageLimit)} · {(100 - balance.usagePercentage).toFixed(1)}% 剩余
                  </span>
                </span>
              ) : cachedBalance && cachedBalance.ttlSecs > 0 && cachedBalance.usageLimit > 0 ? (
                <span className="inline-flex flex-col gap-0.5 ml-1 align-top">
                  <span className={`font-medium tabular-nums ${cachedBalance.remaining < 0 ? 'text-destructive' : 'text-green-600'}`}>
                    {formatKiroCredits(cachedBalance.remaining)} / {formatKiroCredits(cachedBalance.usageLimit)}
                  </span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    ≈ {formatKiroCreditsAsUsd(cachedBalance.remaining)} / {formatKiroCreditsAsUsd(cachedBalance.usageLimit)} · {(100 - cachedBalance.usagePercentage).toFixed(1)}% 剩余 · {formatCacheAge(cachedBalance.cachedAt)}缓存
                  </span>
                </span>
              ) : cachedBalance && cachedBalance.ttlSecs > 0 ? (
                <span className="inline-flex flex-col gap-0.5 ml-1 align-top">
                  <span className={`font-medium tabular-nums ${cachedBalance.remaining < 0 ? 'text-destructive' : 'text-green-600'}`}>{formatKiroCredits(cachedBalance.remaining)}</span>
                  <span className="text-xs text-muted-foreground tabular-nums">
                    ≈ {formatKiroCreditsAsUsd(cachedBalance.remaining)} · {formatCacheAge(cachedBalance.cachedAt)}缓存
                  </span>
                </span>
              ) : (
                <span className="text-sm text-muted-foreground ml-1">未知</span>
              )}
            </div>
            {credential.hasProxy && (
              <div className="col-span-2">
                <span className="text-muted-foreground">代理：</span>
                <span className="font-medium">{credential.proxyUrl}</span>
              </div>
            )}
            <div className="col-span-2">
              <span className="text-muted-foreground">Endpoint：</span>
              {editingEndpoint ? (
                <div className="inline-flex items-center gap-1 ml-1 flex-wrap">
                  <select
                    value={endpointValue}
                    onChange={(e) => setEndpointValue(e.target.value)}
                    className="flex h-7 rounded-md border border-input bg-background px-2 py-1 text-sm ring-offset-background focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                    aria-label="Endpoint"
                  >
                    <option value="">默认值</option>
                    <option value="ide">ide</option>
                    <option value="cli">cli</option>
                  </select>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 w-7 p-0"
                    onClick={handleEndpointChange}
                    disabled={setEndpoint.isPending}
                    aria-label="确认"
                  >
                    <Check className="h-4 w-4" aria-hidden="true" />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 w-7 p-0"
                    onClick={() => {
                      setEditingEndpoint(false)
                      setEndpointValue(credential.endpoint ?? '')
                    }}
                    aria-label="取消"
                  >
                    <X className="h-4 w-4" aria-hidden="true" />
                  </Button>
                </div>
              ) : (
                <button
                  type="button"
                  className="group inline-flex items-center gap-1 font-medium ml-1 text-left bg-transparent border-0 p-0 hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring rounded-sm"
                  onClick={() => {
                    setEndpointValue(credential.endpoint ?? '')
                    setEditingEndpoint(true)
                  }}
                  aria-label={`编辑 Endpoint（当前 ${credential.endpoint || '默认值'}，生效 ${credential.effectiveEndpoint}）`}
                >
                  <span>{credential.endpoint || '默认值'}</span>
                  <span className="text-xs text-muted-foreground font-normal">
                    (生效: {credential.effectiveEndpoint})
                  </span>
                  <Pencil className="h-3 w-3 text-muted-foreground opacity-50 group-hover:opacity-100 transition-opacity" aria-hidden="true" />
                </button>
              )}
            </div>
            {/* Region 配置 */}
            <div className="col-span-2">
              <span className="text-muted-foreground">Region：</span>
              {editingRegion ? (
                <div className="inline-flex items-center gap-1 ml-1 flex-wrap">
                  <Input
                    placeholder="Region（留空清除）"
                    value={regionValue}
                    onChange={(e) => setRegionValue(e.target.value)}
                    className="w-32 h-7 text-sm"
                    aria-label="Region"
                  />
                  <Input
                    placeholder="API Region（可选）"
                    value={apiRegionValue}
                    onChange={(e) => setApiRegionValue(e.target.value)}
                    className="w-36 h-7 text-sm"
                    aria-label="API Region"
                  />
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 w-7 p-0"
                    onClick={handleRegionChange}
                    disabled={setRegion.isPending}
                    aria-label="确认"
                  >
                    <Check className="h-4 w-4" aria-hidden="true" />
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 w-7 p-0"
                    onClick={() => {
                      setEditingRegion(false)
                      setRegionValue(credential.region ?? '')
                      setApiRegionValue(credential.apiRegion ?? '')
                    }}
                    aria-label="取消"
                  >
                    <X className="h-4 w-4" aria-hidden="true" />
                  </Button>
                </div>
              ) : (
                <button
                  type="button"
                  className="group inline-flex items-center gap-1 font-medium ml-1 text-left bg-transparent border-0 p-0 hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring rounded-sm"
                  onClick={() => {
                    setRegionValue(credential.region ?? '')
                    setApiRegionValue(credential.apiRegion ?? '')
                    setEditingRegion(true)
                  }}
                  aria-label={`编辑 Region（当前 ${credential.region || '全局默认'}${credential.apiRegion ? `, API ${credential.apiRegion}` : ''}）`}
                >
                  <span>{credential.region || '全局默认'}</span>
                  {credential.apiRegion && (
                    <span className="text-muted-foreground font-normal">
                      / API: {credential.apiRegion}
                    </span>
                  )}
                  <Pencil className="h-3 w-3 text-muted-foreground opacity-50 group-hover:opacity-100 transition-opacity" aria-hidden="true" />
                </button>
              )}
            </div>
            {credential.hasProfileArn && (
              <div className="col-span-2">
                <Badge variant="secondary">有 Profile ARN</Badge>
              </div>
            )}
          </div>

          {/* 操作按钮 */}
          <div className="flex flex-wrap gap-2 pt-2 border-t">
            <Button
              size="sm"
              variant="outline"
              onClick={handleReset}
              disabled={resetFailure.isPending || (credential.failureCount === 0 && credential.refreshFailureCount === 0)}
            >
              <RotateCcw className="h-4 w-4 mr-1" aria-hidden="true" />
              重置失败
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={handleForceRefresh}
              disabled={forceRefreshToken.isPending || credential.disabled || credential.authMethod === 'api_key'}
              title={credential.authMethod === 'api_key' ? 'API Key 凭据无需刷新 Token' : credential.disabled ? '已禁用的凭据无法刷新 Token' : '强制刷新 Token'}
            >
              <RefreshCw className={`h-4 w-4 mr-1 ${forceRefreshToken.isPending ? 'animate-spin' : ''}`} aria-hidden="true" />
              刷新 Token
            </Button>
            <Button
              size="sm"
              variant="default"
              onClick={handleViewBalance}
            >
              <Wallet className="h-4 w-4 mr-1" aria-hidden="true" />
              查看余额
            </Button>
            <Button
              size="sm"
              variant="destructive"
              onClick={() => setShowDeleteDialog(true)}
              disabled={!credential.disabled}
              title={!credential.disabled ? '需要先禁用凭据才能删除' : undefined}
            >
              <Trash2 className="h-4 w-4 mr-1" aria-hidden="true" />
              删除
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* 删除确认对话框 */}
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
