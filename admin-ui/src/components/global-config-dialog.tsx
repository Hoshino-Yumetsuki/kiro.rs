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
import { Switch } from '@/components/ui/switch'
import {
  useProxyConfig,
  useUpdateProxyConfig,
  useGlobalConfig,
  useUpdateGlobalConfig,
} from '@/hooks/use-credentials'
import type { UpdateGlobalConfigRequest, UpdateCompressionConfigRequest, UpdateRewriterConfigRequest } from '@/types/api'

interface GlobalConfigDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function GlobalConfigDialog({ open, onOpenChange }: GlobalConfigDialogProps) {
  const { data: proxyConfig, isLoading: proxyLoading } = useProxyConfig()
  const { data: globalConfig, isLoading: globalLoading } = useGlobalConfig()
  const { mutate: mutateProxy, isPending: proxyPending } = useUpdateProxyConfig()
  const { mutate: mutateGlobal, isPending: globalPending } = useUpdateGlobalConfig()

  // 基本设置
  const [region, setRegion] = useState('')
  const [credentialRpm, setCredentialRpm] = useState('')
  const [promptCacheTtlSeconds, setPromptCacheTtlSeconds] = useState('300')
  const [promptCacheAccountingEnabled, setPromptCacheAccountingEnabled] = useState(true)
  const [defaultEndpoint, setDefaultEndpoint] = useState('ide')
  const [enableCredentialCooldown, setEnableCredentialCooldown] = useState(true)
  const [enableStickyRouting, setEnableStickyRouting] = useState(true)
  const [autoDisableInsufficientBalance, setAutoDisableInsufficientBalance] = useState(true)
  const [autoDisableRefreshFailure, setAutoDisableRefreshFailure] = useState(true)

  // 代理设置
  const [proxyUrl, setProxyUrl] = useState('')
  const [proxyUsername, setProxyUsername] = useState('')
  const [proxyPassword, setProxyPassword] = useState('')

  // 压缩配置
  const [cEnabled, setCEnabled] = useState(true)
  const [cWhitespace, setCWhitespace] = useState(true)
  const [cThinkingStrategy, setCThinkingStrategy] = useState('discard')
  const [cToolDescMaxChars, setCToolDescMaxChars] = useState('')
  const [cToolDefCompression, setCToolDefCompression] = useState(false)
  const [cToolDefMinDescChars, setCToolDefMinDescChars] = useState('')
  const [cToolNameMaxChars, setCToolNameMaxChars] = useState('')
  const [cMaxRequestBodyBytes, setCMaxRequestBodyBytes] = useState('')
  const [cAdaptive, setCAdaptive] = useState(false)
  const [cAdaptiveMaxIters, setCAdaptiveMaxIters] = useState('')

  // 改写配置
  const [rewriterEnabled, setRewriterEnabled] = useState(false)

  const isLoading = proxyLoading || globalLoading
  const isPending = proxyPending || globalPending

  useEffect(() => {
    if (open && globalConfig) {
      setRegion(globalConfig.region || '')
      setCredentialRpm(globalConfig.credentialRpm?.toString() || '')
      setPromptCacheTtlSeconds(globalConfig.promptCacheTtlSeconds.toString())
      setPromptCacheAccountingEnabled(globalConfig.promptCacheAccountingEnabled)
      setDefaultEndpoint(globalConfig.defaultEndpoint || 'ide')
      setEnableCredentialCooldown(globalConfig.enableCredentialCooldown)
      setEnableStickyRouting(globalConfig.enableStickyRouting)
      setAutoDisableInsufficientBalance(globalConfig.autoDisableInsufficientBalance)
      setAutoDisableRefreshFailure(globalConfig.autoDisableRefreshFailure)
      const c = globalConfig.compression
      setCEnabled(c.enabled)
      setCWhitespace(c.whitespaceCompression)
      setCThinkingStrategy(c.thinkingStrategy)
      setCToolDescMaxChars(c.toolDescriptionMaxChars.toString())
      setCToolDefCompression(c.toolDefinitionCompression)
      setCToolDefMinDescChars(c.toolDefinitionMinDescriptionChars.toString())
      setCToolNameMaxChars(c.toolNameMaxChars.toString())
      setCMaxRequestBodyBytes(c.maxRequestBodyBytes.toString())
      setCAdaptive(c.adaptiveCompression)
      setCAdaptiveMaxIters(c.adaptiveCompressionMaxIters.toString())
      // 改写配置
      setRewriterEnabled(globalConfig.rewriter.enabled)
    }
    if (open && proxyConfig) {
      setProxyUrl(proxyConfig.proxyUrl || '')
      setProxyUsername('')
      setProxyPassword('')
    }
  }, [open, globalConfig, proxyConfig])

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()

    const globalPayload: UpdateGlobalConfigRequest = {}
    let hasGlobalChanges = false

    if (region.trim() !== (globalConfig?.region || '')) {
      globalPayload.region = region.trim()
      hasGlobalChanges = true
    }

    const newRpm = credentialRpm.trim() ? parseInt(credentialRpm.trim(), 10) : null
    if (newRpm !== (globalConfig?.credentialRpm ?? null)) {
      globalPayload.credentialRpm = newRpm
      hasGlobalChanges = true
    }

    const newPromptCacheTtlSeconds = parseInt(promptCacheTtlSeconds, 10)
    if (globalConfig && newPromptCacheTtlSeconds !== globalConfig.promptCacheTtlSeconds) {
      globalPayload.promptCacheTtlSeconds = newPromptCacheTtlSeconds
      hasGlobalChanges = true
    }

    if (globalConfig && promptCacheAccountingEnabled !== globalConfig.promptCacheAccountingEnabled) {
      globalPayload.promptCacheAccountingEnabled = promptCacheAccountingEnabled
      hasGlobalChanges = true
    }

    if (defaultEndpoint !== (globalConfig?.defaultEndpoint || 'ide')) {
      globalPayload.defaultEndpoint = defaultEndpoint
      hasGlobalChanges = true
    }

    if (enableCredentialCooldown !== (globalConfig?.enableCredentialCooldown ?? true)) {
      globalPayload.enableCredentialCooldown = enableCredentialCooldown
      hasGlobalChanges = true
    }

    if (enableStickyRouting !== (globalConfig?.enableStickyRouting ?? true)) {
      globalPayload.enableStickyRouting = enableStickyRouting
      hasGlobalChanges = true
    }

    if (autoDisableInsufficientBalance !== (globalConfig?.autoDisableInsufficientBalance ?? true)) {
      globalPayload.autoDisableInsufficientBalance = autoDisableInsufficientBalance
      hasGlobalChanges = true
    }

    if (autoDisableRefreshFailure !== (globalConfig?.autoDisableRefreshFailure ?? true)) {
      globalPayload.autoDisableRefreshFailure = autoDisableRefreshFailure
      hasGlobalChanges = true
    }

    // 构建压缩配置 diff
    if (globalConfig) {
      const oc = globalConfig.compression
      const comp: UpdateCompressionConfigRequest = {}
      let hasCompChanges = false
      const setIf = <K extends keyof UpdateCompressionConfigRequest>(
        key: K, newVal: UpdateCompressionConfigRequest[K], oldVal: UpdateCompressionConfigRequest[K]
      ) => {
        if (newVal !== oldVal) { comp[key] = newVal; hasCompChanges = true }
      }
      setIf('enabled', cEnabled, oc.enabled)
      setIf('whitespaceCompression', cWhitespace, oc.whitespaceCompression)
      setIf('thinkingStrategy', cThinkingStrategy, oc.thinkingStrategy)
      setIf('toolDescriptionMaxChars', parseInt(cToolDescMaxChars) || 0, oc.toolDescriptionMaxChars)
      setIf('toolDefinitionCompression', cToolDefCompression, oc.toolDefinitionCompression)
      setIf('toolDefinitionMinDescriptionChars', parseInt(cToolDefMinDescChars) || 0, oc.toolDefinitionMinDescriptionChars)
      setIf('toolNameMaxChars', parseInt(cToolNameMaxChars) || 0, oc.toolNameMaxChars)
      setIf('maxRequestBodyBytes', parseInt(cMaxRequestBodyBytes) || 0, oc.maxRequestBodyBytes)
      setIf('adaptiveCompression', cAdaptive, oc.adaptiveCompression)
      setIf('adaptiveCompressionMaxIters', parseInt(cAdaptiveMaxIters) || 0, oc.adaptiveCompressionMaxIters)
      if (hasCompChanges) {
        globalPayload.compression = comp
        hasGlobalChanges = true
      }

      // 改写配置 diff
      const rewriter: UpdateRewriterConfigRequest = {}
      let hasRewriterChanges = false
      if (rewriterEnabled !== globalConfig.rewriter.enabled) {
        rewriter.enabled = rewriterEnabled
        hasRewriterChanges = true
      }
      if (hasRewriterChanges) {
        globalPayload.rewriter = rewriter
        hasGlobalChanges = true
      }
    }

    // 代理配置
    const proxyPayload: Record<string, string | null> = {
      proxyUrl: proxyUrl.trim() || null,
    }
    if (proxyUsername.trim() || proxyPassword.trim()) {
      proxyPayload.proxyUsername = proxyUsername.trim() || null
      proxyPayload.proxyPassword = proxyPassword.trim() || null
    }
    const hasProxyChanges =
      proxyPayload.proxyUrl !== (proxyConfig?.proxyUrl || null) ||
      proxyPayload.proxyUsername !== undefined ||
      proxyPayload.proxyPassword !== undefined

    let pending = 0
    let hasError = false
    const done = () => {
      pending--
      if (pending <= 0 && !hasError) onOpenChange(false)
    }
    const fail = () => {
      hasError = true
      pending--
    }

    if (hasGlobalChanges) {
      pending++
      mutateGlobal(globalPayload, { onSuccess: done, onError: fail })
    }
    if (hasProxyChanges) {
      pending++
      mutateProxy(proxyPayload as never, { onSuccess: done, onError: fail })
    }
    if (pending === 0) onOpenChange(false)
  }

  const numInput = (id: string, label: string, value: string, setter: (v: string) => void, hint?: string) => (
    <div className="space-y-1">
      <label htmlFor={id} className="text-sm font-medium">{label}</label>
      <Input id={id} type="number" min={0} value={value} onChange={(e) => setter(e.target.value)} disabled={isPending} />
      {hint && <p className="text-xs text-muted-foreground">{hint}</p>}
    </div>
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg max-h-[85vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>全局配置</DialogTitle>
        </DialogHeader>

        {isLoading ? (
          <div className="py-8 text-center text-muted-foreground">加载中…</div>
        ) : (
          <form onSubmit={handleSubmit} className="space-y-6">
            {/* 基本设置 */}
            <div className="space-y-3">
              <h3 className="text-sm font-semibold text-muted-foreground">基本设置</h3>
              <div className="space-y-1">
                <label htmlFor="gcRegion" className="text-sm font-medium">Region</label>
                <Input id="gcRegion" placeholder="us-east-1" value={region} onChange={(e) => setRegion(e.target.value)} disabled={isPending} />
              </div>
              {numInput('gcRpm', 'Credential RPM', credentialRpm, setCredentialRpm, '单凭据每分钟请求数上限，0 或留空使用默认策略')}
              <div className="space-y-1">
                <label htmlFor="gcPromptCacheTtl" className="text-sm font-medium">Prompt Cache TTL</label>
                <select
                  id="gcPromptCacheTtl"
                  className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                  value={promptCacheTtlSeconds}
                  onChange={(e) => setPromptCacheTtlSeconds(e.target.value)}
                  disabled={isPending}
                >
                  <option value="300">5 分钟</option>
                  <option value="3600">1 小时</option>
                </select>
              </div>
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">Prompt Cache 记账</label>
                <Switch checked={promptCacheAccountingEnabled} onCheckedChange={setPromptCacheAccountingEnabled} disabled={isPending} aria-label="Prompt Cache 记账" />
              </div>
              <div className="space-y-1">
                <label htmlFor="gcDefaultEndpoint" className="text-sm font-medium">默认端点</label>
                <Input id="gcDefaultEndpoint" placeholder="ide" value={defaultEndpoint} onChange={(e) => setDefaultEndpoint(e.target.value)} disabled={isPending} />
              </div>
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">凭据冷却机制</label>
                <Switch checked={enableCredentialCooldown} onCheckedChange={setEnableCredentialCooldown} disabled={isPending} aria-label="凭据冷却机制" />
              </div>
              <div className="flex items-center justify-between">
                <div>
                  <label className="text-sm font-medium">渠道亲和</label>
                </div>
                <Switch checked={enableStickyRouting} onCheckedChange={setEnableStickyRouting} disabled={isPending} aria-label="渠道亲和" />
              </div>
              <div className="flex items-center justify-between">
                <div>
                  <label className="text-sm font-medium">余额不足自动禁用</label>
                  <p className="text-xs text-muted-foreground">余额初始化时检测到不足将自动禁用凭据</p>
                </div>
                <Switch checked={autoDisableInsufficientBalance} onCheckedChange={setAutoDisableInsufficientBalance} disabled={isPending} aria-label="余额不足自动禁用" />
              </div>
              <div className="flex items-center justify-between">
                <div>
                  <label className="text-sm font-medium">刷新失败自动禁用</label>
                  <p className="text-xs text-muted-foreground">Token 刷新连续失败达阈值时自动禁用凭据</p>
                </div>
                <Switch checked={autoDisableRefreshFailure} onCheckedChange={setAutoDisableRefreshFailure} disabled={isPending} aria-label="刷新失败自动禁用" />
              </div>
            </div>

            {/* 代理设置 */}
            <div className="space-y-3">
              <h3 className="text-sm font-semibold text-muted-foreground">代理设置</h3>
              <div className="space-y-1">
                <label htmlFor="gcProxyUrl" className="text-sm font-medium">代理 URL</label>
                <Input id="gcProxyUrl" placeholder="http://proxy:8080 或 socks5://proxy:1080" value={proxyUrl} onChange={(e) => setProxyUrl(e.target.value)} disabled={isPending} />
                <p className="text-xs text-muted-foreground">留空不使用全局代理，凭据级代理优先</p>
              </div>
              <div className="space-y-1">
                <label className="text-sm font-medium">代理认证（可选）</label>
                <div className="grid grid-cols-2 gap-2">
                  <div>
                    <label htmlFor="gcProxyUsername" className="sr-only">代理用户名</label>
                    <Input id="gcProxyUsername" placeholder="用户名" value={proxyUsername} onChange={(e) => setProxyUsername(e.target.value)} disabled={isPending} />
                  </div>
                  <div>
                    <label htmlFor="gcProxyPassword" className="sr-only">代理密码</label>
                    <Input id="gcProxyPassword" type="password" placeholder="密码" value={proxyPassword} onChange={(e) => setProxyPassword(e.target.value)} disabled={isPending} />
                  </div>
                </div>
                {proxyConfig?.hasCredentials && <p className="text-xs text-muted-foreground">已配置认证，留空保持不变</p>}
              </div>
            </div>

            {/* 压缩配置 */}
            <div className="space-y-3">
              <h3 className="text-sm font-semibold text-muted-foreground">压缩配置</h3>

              {/* 基本压缩 */}
              <p className="text-xs font-medium text-muted-foreground pt-1">基本压缩</p>
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">启用压缩</label>
                <Switch checked={cEnabled} onCheckedChange={setCEnabled} disabled={isPending} aria-label="启用压缩" />
              </div>
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">空白压缩</label>
                <Switch checked={cWhitespace} onCheckedChange={setCWhitespace} disabled={isPending} aria-label="空白压缩" />
              </div>
              <div className="space-y-1">
                <label htmlFor="gcThinking" className="text-sm font-medium">Thinking 策略</label>
                <select
                  id="gcThinking"
                  className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                  value={cThinkingStrategy}
                  onChange={(e) => setCThinkingStrategy(e.target.value)}
                  disabled={isPending}
                >
                  <option value="discard">discard</option>
                  <option value="truncate">truncate</option>
                  <option value="keep">keep</option>
                </select>
              </div>

              {/* 工具定义 */}
              <p className="text-xs font-medium text-muted-foreground pt-2">工具定义</p>
              {numInput('gcTdMaxChars', '工具描述截断阈值（字符）', cToolDescMaxChars, setCToolDescMaxChars)}
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">工具定义压缩</label>
                <Switch checked={cToolDefCompression} onCheckedChange={setCToolDefCompression} disabled={isPending} aria-label="工具定义压缩" />
              </div>
              {numInput('gcTdMinDescChars', '工具定义最小描述字符数', cToolDefMinDescChars, setCToolDefMinDescChars)}
              {numInput('gcTnMaxChars', '工具名称最大字符数', cToolNameMaxChars, setCToolNameMaxChars, '0 = 不限')}

              {/* 自适应压缩 */}
              <p className="text-xs font-medium text-muted-foreground pt-2">自适应压缩</p>
              {numInput('gcMaxBody', '请求体大小上限（字节）', cMaxRequestBodyBytes, setCMaxRequestBodyBytes, '超过此大小触发自适应压缩，0 = 不限')}
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">启用自适应压缩</label>
                <Switch checked={cAdaptive} onCheckedChange={setCAdaptive} disabled={isPending} aria-label="启用自适应压缩" />
              </div>
              {numInput('gcAdaptiveMaxIters', '最大迭代次数', cAdaptiveMaxIters, setCAdaptiveMaxIters)}
            </div>

            {/* 响应改写 */}
            <div className="space-y-3">
              <h3 className="text-sm font-semibold text-muted-foreground">响应改写</h3>
              <div className="flex items-center justify-between">
                <div>
                  <label className="text-sm font-medium">启用响应改写</label>
                  <p className="text-xs text-muted-foreground">检测到自我认知关键词时，调用模型改写为 Claude 身份</p>
                </div>
                <Switch checked={rewriterEnabled} onCheckedChange={setRewriterEnabled} disabled={isPending} aria-label="启用响应改写" />
              </div>
            </div>

            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>取消</Button>
              <Button type="submit" disabled={isPending}>{isPending ? `保存中…` : '保存'}</Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  )
}
