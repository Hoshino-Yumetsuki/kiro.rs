import { useState, useEffect } from 'react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useProxyConfig, useUpdateProxyConfig } from '@/hooks/use-credentials'

interface ProxyConfigDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function ProxyConfigDialog({ open, onOpenChange }: ProxyConfigDialogProps) {
  const { data: config, isLoading } = useProxyConfig()
  const { mutate, isPending } = useUpdateProxyConfig()

  const [proxyUrl, setProxyUrl] = useState('')
  const [proxyUsername, setProxyUsername] = useState('')
  const [proxyPassword, setProxyPassword] = useState('')

  // 当配置加载完成或对话框打开时，同步表单状态
  useEffect(() => {
    if (open && config) {
      setProxyUrl(config.proxyUrl || '')
      setProxyUsername('')
      setProxyPassword('')
    }
  }, [open, config])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    const payload: Record<string, string | null> = {
      proxyUrl: proxyUrl.trim() || null,
    }
    // 仅当用户填写了认证信息时才发送，留空则保留后端现有认证
    if (proxyUsername.trim() || proxyPassword.trim()) {
      payload.proxyUsername = proxyUsername.trim() || null
      payload.proxyPassword = proxyPassword.trim() || null
    }

    mutate(payload, {
      onSuccess: () => {
        onOpenChange(false)
      },
    })
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>全局代理配置</DialogTitle>
        </DialogHeader>

        {isLoading ? (
          <div className="py-8 text-center text-muted-foreground">加载中…</div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <label htmlFor="globalProxyUrl" className="text-sm font-medium">
                代理 URL
              </label>
              <Input
                id="globalProxyUrl"
                placeholder="例如 http://proxy:8080 或 socks5://proxy:1080"
                value={proxyUrl}
                onChange={(e) => setProxyUrl(e.target.value)}
                disabled={isPending}
              />
              <p className="text-xs text-muted-foreground">
                留空表示不使用全局代理。凭据级代理优先于全局代理
              </p>
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">代理认证（可选）</label>
              <div className="grid grid-cols-2 gap-2">
                <Input
                  id="globalProxyUsername"
                  placeholder="用户名"
                  value={proxyUsername}
                  onChange={(e) => setProxyUsername(e.target.value)}
                  disabled={isPending}
                />
                <Input
                  id="globalProxyPassword"
                  type="password"
                  placeholder="密码"
                  value={proxyPassword}
                  onChange={(e) => setProxyPassword(e.target.value)}
                  disabled={isPending}
                />
              </div>
              {config?.hasCredentials && (
                <p className="text-xs text-muted-foreground">
                  已配置代理认证。留空保持不变，填写则覆盖
                </p>
              )}
            </div>

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
                disabled={isPending}
              >
                取消
              </Button>
              <Button type="submit" disabled={isPending}>
                {isPending ? `保存中…` : '保存'}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}
