import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { AlertTriangle } from "lucide-react";
import { Progress } from "@/components/ui/progress";
import { useCredentialBalance } from "@/hooks/use-credentials";
import { formatKiroCredits, formatKiroCreditsAsUsd } from "@/lib/format";
import { cn, parseError } from "@/lib/utils";

interface BalanceDialogProps {
  credentialId: number | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  forceRefresh?: boolean;
}

export function BalanceDialog({
  credentialId,
  open,
  onOpenChange,
  forceRefresh,
}: BalanceDialogProps) {
  const { data: balance, isLoading, isFetching, error } = useCredentialBalance(credentialId);
  const showLoading = isLoading || (forceRefresh && isFetching);

  const formatDate = (timestamp: number | null) => {
    if (!timestamp) return "未知";
    return new Date(timestamp * 1000).toLocaleString("zh-CN");
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>凭据 #{credentialId} 余额信息</DialogTitle>
          <DialogDescription>
            以 Kiro credits 为主，美元金额按 1 credit = $0.04 估算。
          </DialogDescription>
        </DialogHeader>

        {showLoading && (
          <div
            className="flex items-center justify-center py-8"
            aria-live="polite"
            aria-busy="true"
          >
            <div
              className="animate-spin rounded-full h-8 w-8 border-b-2 border-primary"
              aria-hidden="true"
            ></div>
          </div>
        )}

        {error &&
          (() => {
            const parsed = parseError(error);
            return (
              <div className="py-6 space-y-3">
                <div className="flex items-center justify-center gap-2 text-red-500">
                  <svg
                    className="h-5 w-5"
                    viewBox="0 0 20 20"
                    fill="currentColor"
                    aria-hidden="true"
                  >
                    <path
                      fillRule="evenodd"
                      d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z"
                      clipRule="evenodd"
                    />
                  </svg>
                  <span className="font-medium">{parsed.title}</span>
                </div>
                {parsed.detail && (
                  <div className="text-sm text-muted-foreground text-center px-4">
                    {parsed.detail}
                  </div>
                )}
              </div>
            );
          })()}

        {balance &&
          (() => {
            const overspentCredits = Math.max(
              balance.currentUsage - balance.usageLimit,
              -balance.remaining,
              0,
            );
            const isOverspent = overspentCredits > 0;
            const displayedRemaining = isOverspent ? -overspentCredits : balance.remaining;

            return (
              <div className="space-y-4">
                {/* 订阅类型 */}
                <div className="text-center">
                  <span className="text-lg font-semibold">
                    {balance.subscriptionTitle || "未知订阅类型"}
                  </span>
                </div>

                {/* 使用进度 */}
                <div className="space-y-2">
                  <div className="grid grid-cols-2 gap-3 text-sm">
                    <div>
                      <div className="text-muted-foreground">已使用</div>
                      <div className="font-medium tabular-nums">
                        {formatKiroCredits(balance.currentUsage)}
                      </div>
                      <div className="text-xs text-muted-foreground tabular-nums">
                        ≈ {formatKiroCreditsAsUsd(balance.currentUsage)}
                      </div>
                    </div>
                    <div className="text-right">
                      <div className="text-muted-foreground">限额</div>
                      <div className="font-medium tabular-nums">
                        {formatKiroCredits(balance.usageLimit)}
                      </div>
                      <div className="text-xs text-muted-foreground tabular-nums">
                        ≈ {formatKiroCreditsAsUsd(balance.usageLimit)}
                      </div>
                    </div>
                  </div>
                  <Progress value={balance.usagePercentage} />
                  <div
                    className={cn(
                      "text-center text-sm",
                      isOverspent ? "font-medium text-destructive" : "text-muted-foreground",
                    )}
                  >
                    {balance.usagePercentage.toFixed(1)}% 已使用
                  </div>
                  {isOverspent && (
                    <div className="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
                      <AlertTriangle className="mt-0.5 h-4 w-4 flex-shrink-0" aria-hidden="true" />
                      <div>
                        <div className="font-medium">
                          已超出限额 {formatKiroCredits(overspentCredits)}
                        </div>
                        <div className="text-xs text-destructive/80">
                          Kiro 支持 overspending，因此该账号仍可显示为超支状态。
                        </div>
                      </div>
                    </div>
                  )}
                </div>

                {/* 详细信息 */}
                <div className="grid grid-cols-2 gap-4 pt-4 border-t text-sm">
                  <div>
                    <span className="text-muted-foreground">
                      {isOverspent ? "已超支：" : "剩余额度："}
                    </span>
                    <div
                      className={cn(
                        "font-medium tabular-nums",
                        isOverspent ? "text-destructive" : "text-green-600",
                      )}
                    >
                      {formatKiroCredits(displayedRemaining)}
                    </div>
                    <div className="text-xs text-muted-foreground tabular-nums">
                      ≈ {formatKiroCreditsAsUsd(displayedRemaining)}
                    </div>
                  </div>
                  <div>
                    <span className="text-muted-foreground">下次重置：</span>
                    <span className="font-medium">{formatDate(balance.nextResetAt)}</span>
                  </div>
                </div>
              </div>
            );
          })()}
      </DialogContent>
    </Dialog>
  );
}
