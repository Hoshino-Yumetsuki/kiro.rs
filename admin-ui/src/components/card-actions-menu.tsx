import { MoreHorizontal, Pencil, MapPin, Globe, RotateCcw, RefreshCw, Wallet, Trash2, Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from '@/components/ui/dropdown-menu'
import {
  Tooltip,
  TooltipTrigger,
  TooltipContent,
} from '@/components/ui/tooltip'
import type { CredentialStatusItem } from '@/types/api'

export interface CardActionsMenuProps {
  credential: CredentialStatusItem
  onResetFailures: () => void
  onRefreshToken: () => void
  onViewBalance: () => void
  onDelete: () => void
  onEditPriority: () => void
  onEditRegion: () => void
  onEditEndpoint: () => void
  isResetting?: boolean
  isRefreshing?: boolean
  isDeleting?: boolean
}

/**
 * 凭据卡片操作菜单（DropdownMenu 溢出菜单）
 *
 * Disabled 逻辑（与 credential-card.tsx 同步）：
 * - 重置失败：failureCount === 0 && refreshFailureCount === 0
 * - 刷新 Token：authMethod === 'api_key'
 * - 删除：disabled === false（凭据未禁用时不能删除）
 */
export function CardActionsMenu({
  credential,
  onResetFailures,
  onRefreshToken,
  onViewBalance,
  onDelete,
  onEditPriority,
  onEditRegion,
  onEditEndpoint,
  isResetting = false,
  isRefreshing = false,
  isDeleting = false,
}: CardActionsMenuProps) {
  const resetDisabled = credential.failureCount === 0 && credential.refreshFailureCount === 0
  const refreshDisabled = credential.authMethod === 'api_key'
  const deleteDisabled = !credential.disabled

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon" aria-label="凭据操作菜单">
          <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48">
        {/* ====== 编辑组 ====== */}
        <DropdownMenuItem onClick={onEditPriority}>
          <Pencil className="h-4 w-4" aria-hidden="true" />
          编辑优先级
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onEditRegion}>
          <MapPin className="h-4 w-4" aria-hidden="true" />
          编辑区域
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onEditEndpoint}>
          <Globe className="h-4 w-4" aria-hidden="true" />
          编辑端点
        </DropdownMenuItem>

        <DropdownMenuSeparator />

        {/* ====== 操作组 ====== */}
        <Tooltip open={resetDisabled ? undefined : false}>
          <TooltipTrigger asChild>
            <span tabIndex={resetDisabled ? 0 : undefined}>
              <DropdownMenuItem
                onClick={onResetFailures}
                disabled={resetDisabled || isResetting}
              >
                {isResetting ? (
                  <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                ) : (
                  <RotateCcw className="h-4 w-4" aria-hidden="true" />
                )}
                重置失败
              </DropdownMenuItem>
            </span>
          </TooltipTrigger>
          {resetDisabled && (
            <TooltipContent side="right">
              没有失败的记录需要重置
            </TooltipContent>
          )}
        </Tooltip>

        <Tooltip open={refreshDisabled ? undefined : false}>
          <TooltipTrigger asChild>
            <span tabIndex={refreshDisabled ? 0 : undefined}>
              <DropdownMenuItem
                onClick={onRefreshToken}
                disabled={refreshDisabled || isRefreshing}
              >
                {isRefreshing ? (
                  <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                ) : (
                  <RefreshCw className="h-4 w-4" aria-hidden="true" />
                )}
                刷新 Token
              </DropdownMenuItem>
            </span>
          </TooltipTrigger>
          {refreshDisabled && (
            <TooltipContent side="right">
              API Key 凭据不支持刷新 Token
            </TooltipContent>
          )}
        </Tooltip>

        <DropdownMenuItem onClick={onViewBalance}>
          <Wallet className="h-4 w-4" aria-hidden="true" />
          查看余额
        </DropdownMenuItem>

        <DropdownMenuSeparator />

        {/* ====== 危险组 ====== */}
        <Tooltip open={deleteDisabled ? undefined : false}>
          <TooltipTrigger asChild>
            <span tabIndex={deleteDisabled ? 0 : undefined}>
              <DropdownMenuItem
                onClick={onDelete}
                disabled={deleteDisabled || isDeleting}
                className="text-destructive focus:text-destructive focus:bg-destructive/10"
              >
                {isDeleting ? (
                  <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                ) : (
                  <Trash2 className="h-4 w-4" aria-hidden="true" />
                )}
                删除
              </DropdownMenuItem>
            </span>
          </TooltipTrigger>
          {deleteDisabled && (
            <TooltipContent side="right">
              需要先禁用凭据才能删除
            </TooltipContent>
          )}
        </Tooltip>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
