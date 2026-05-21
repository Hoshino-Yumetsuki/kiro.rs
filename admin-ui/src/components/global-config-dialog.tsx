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
import type { UpdateGlobalConfigRequest, UpdateCompressionConfigRequest } from '@/types/api'

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

  // 代理设置
  const [proxyUrl, setProxyUrl] = useState('')
  const [proxyUsername, setProxyUsername] = useState('')
  const [proxyPassword, setProxyPassword] = useState('')

  // 压缩配置
  const [cEnabled, setCEnabled] = useState(true)
  const [cWhitespace, setCWhitespace] = useState(true)
  const [cThinkingStrategy, setCThinkingStrategy] = useState('discard')
  const [cToolResultMaxChars, setCToolResultMaxChars] = useState('')
  const [cToolResultHeadLines, setCToolResultHeadLines] = useState('')
  const [cToolResultTailLines, setCToolResultTailLines] = useState('')
  const [cToolUseInputMaxChars, setCToolUseInputMaxChars] = useState('')
  const [cToolDescMaxChars, setCToolDescMaxChars] = useState('')
  const [cToolDefCompression, setCToolDefCompression] = useState(false)
  const [cToolDefSizeThreshold, setCToolDefSizeThreshold] = useState('')
  const [cToolDefMinDescChars, setCToolDefMinDescChars] = useState('')
  const [cToolNameMaxChars, setCToolNameMaxChars] = useState('')
  const [cImageMaxLongEdge, setCImageMaxLongEdge] = useState('')
  const [cImageMaxPixelsSingle, setCImageMaxPixelsSingle] = useState('')
  const [cImageMaxPixelsMulti, setCImageMaxPixelsMulti] = useState('')
  const [cImageMultiThreshold, setCImageMultiThreshold] = useState('')
  const [cMaxHistoryTurns, setCMaxHistoryTurns] = useState('')
  const [cMaxHistoryChars, setCMaxHistoryChars] = useState('')
  const [cMaxRequestBodyBytes, setCMaxRequestBodyBytes] = useState('')
  const [cAdaptive, setCAdaptive] = useState(false)
  const [cAdaptiveMaxIters, setCAdaptiveMaxIters] = useState('')
  const [cAdaptiveToolResult, setCAdaptiveToolResult] = useState(false)
  const [cAdaptiveMinToolResultChars, setCAdaptiveMinToolResultChars] = useState('')
  const [cAdaptiveToolUseInput, setCAdaptiveToolUseInput] = useState(false)
  const [cAdaptiveMinToolUseInputChars, setCAdaptiveMinToolUseInputChars] = useState('')
  const [cAdaptiveHistoryImage, setCAdaptiveHistoryImage] = useState(false)
  const [cAdaptiveHistoryRemoval, setCAdaptiveHistoryRemoval] = useState(false)
  const [cAdaptiveHistoryPreserve, setCAdaptiveHistoryPreserve] = useState('')

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
      const c = globalConfig.compression
      setCEnabled(c.enabled)
      setCWhitespace(c.whitespaceCompression)
      setCThinkingStrategy(c.thinkingStrategy)
      setCToolResultMaxChars(c.toolResultMaxChars.toString())
      setCToolResultHeadLines(c.toolResultHeadLines.toString())
      setCToolResultTailLines(c.toolResultTailLines.toString())
      setCToolUseInputMaxChars(c.toolUseInputMaxChars.toString())
      setCToolDescMaxChars(c.toolDescriptionMaxChars.toString())
      setCToolDefCompression(c.toolDefinitionCompression)
      setCToolDefSizeThreshold(c.toolDefinitionSizeThreshold.toString())
      setCToolDefMinDescChars(c.toolDefinitionMinDescriptionChars.toString())
      setCToolNameMaxChars(c.toolNameMaxChars.toString())
      setCImageMaxLongEdge(c.imageMaxLongEdge.toString())
      setCImageMaxPixelsSingle(c.imageMaxPixelsSingle.toString())
      setCImageMaxPixelsMulti(c.imageMaxPixelsMulti.toString())
      setCImageMultiThreshold(c.imageMultiThreshold.toString())
      setCMaxHistoryTurns(c.maxHistoryTurns.toString())
      setCMaxHistoryChars(c.maxHistoryChars.toString())
      setCMaxRequestBodyBytes(c.maxRequestBodyBytes.toString())
      setCAdaptive(c.adaptiveCompression)
      setCAdaptiveMaxIters(c.adaptiveCompressionMaxIters.toString())
      setCAdaptiveToolResult(c.adaptiveToolResultCompression)
      setCAdaptiveMinToolResultChars(c.adaptiveMinToolResultMaxChars.toString())
      setCAdaptiveToolUseInput(c.adaptiveToolUseInputCompression)
      setCAdaptiveMinToolUseInputChars(c.adaptiveMinToolUseInputMaxChars.toString())
      setCAdaptiveHistoryImage(c.adaptiveHistoryImageRemoval)
      setCAdaptiveHistoryRemoval(c.adaptiveHistoryRemoval)
      setCAdaptiveHistoryPreserve(c.adaptiveHistoryPreserveMessages.toString())
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
      setIf('toolResultMaxChars', parseInt(cToolResultMaxChars) || 0, oc.toolResultMaxChars)
      setIf('toolResultHeadLines', parseInt(cToolResultHeadLines) || 0, oc.toolResultHeadLines)
      setIf('toolResultTailLines', parseInt(cToolResultTailLines) || 0, oc.toolResultTailLines)
      setIf('toolUseInputMaxChars', parseInt(cToolUseInputMaxChars) || 0, oc.toolUseInputMaxChars)
      setIf('toolDescriptionMaxChars', parseInt(cToolDescMaxChars) || 0, oc.toolDescriptionMaxChars)
      setIf('toolDefinitionCompression', cToolDefCompression, oc.toolDefinitionCompression)
      setIf('toolDefinitionSizeThreshold', parseInt(cToolDefSizeThreshold) || 0, oc.toolDefinitionSizeThreshold)
      setIf('toolDefinitionMinDescriptionChars', parseInt(cToolDefMinDescChars) || 0, oc.toolDefinitionMinDescriptionChars)
      setIf('toolNameMaxChars', parseInt(cToolNameMaxChars) || 0, oc.toolNameMaxChars)
      setIf('imageMaxLongEdge', parseInt(cImageMaxLongEdge) || 0, oc.imageMaxLongEdge)
      setIf('imageMaxPixelsSingle', parseInt(cImageMaxPixelsSingle) || 0, oc.imageMaxPixelsSingle)
      setIf('imageMaxPixelsMulti', parseInt(cImageMaxPixelsMulti) || 0, oc.imageMaxPixelsMulti)
      setIf('imageMultiThreshold', parseInt(cImageMultiThreshold) || 0, oc.imageMultiThreshold)
      setIf('maxHistoryTurns', parseInt(cMaxHistoryTurns) || 0, oc.maxHistoryTurns)
      setIf('maxHistoryChars', parseInt(cMaxHistoryChars) || 0, oc.maxHistoryChars)
      setIf('maxRequestBodyBytes', parseInt(cMaxRequestBodyBytes) || 0, oc.maxRequestBodyBytes)
      setIf('adaptiveCompression', cAdaptive, oc.adaptiveCompression)
      setIf('adaptiveCompressionMaxIters', parseInt(cAdaptiveMaxIters) || 0, oc.adaptiveCompressionMaxIters)
      setIf('adaptiveToolResultCompression', cAdaptiveToolResult, oc.adaptiveToolResultCompression)
      setIf('adaptiveMinToolResultMaxChars', parseInt(cAdaptiveMinToolResultChars) || 0, oc.adaptiveMinToolResultMaxChars)
      setIf('adaptiveToolUseInputCompression', cAdaptiveToolUseInput, oc.adaptiveToolUseInputCompression)
      setIf('adaptiveMinToolUseInputMaxChars', parseInt(cAdaptiveMinToolUseInputChars) || 0, oc.adaptiveMinToolUseInputMaxChars)
      setIf('adaptiveHistoryImageRemoval', cAdaptiveHistoryImage, oc.adaptiveHistoryImageRemoval)
      setIf('adaptiveHistoryRemoval', cAdaptiveHistoryRemoval, oc.adaptiveHistoryRemoval)
      setIf('adaptiveHistoryPreserveMessages', parseInt(cAdaptiveHistoryPreserve) || 0, oc.adaptiveHistoryPreserveMessages)
      if (hasCompChanges) {
        globalPayload.compression = comp
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
      mutateProxy(proxyPayload, { onSuccess: done, onError: fail })
    }
    if (pending === 0) onOpenChange(false)
  }

  // PLACEHOLDER_JSX

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
                <p className="text-xs text-muted-foreground">仅支持 5 分钟和 1 小时两档，保存后立即生效</p>
              </div>
              <div className="flex items-center justify-between">
                <div className="space-y-1">
                  <label className="text-sm font-medium">Prompt Cache 记账</label>
                  <p className="text-xs text-muted-foreground">关闭后立即停止输出和扣减本地 cache token</p>
                </div>
                <Switch checked={promptCacheAccountingEnabled} onCheckedChange={setPromptCacheAccountingEnabled} disabled={isPending} aria-label="Prompt Cache 记账" />
              </div>
              <div className="space-y-1">
                <label htmlFor="gcDefaultEndpoint" className="text-sm font-medium">默认 Endpoint</label>
                <select
                  id="gcDefaultEndpoint"
                  className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
                  value={defaultEndpoint}
                  onChange={(e) => setDefaultEndpoint(e.target.value)}
                  disabled={isPending}
                >
                  <option value="ide">ide</option>
                  <option value="cli">cli</option>
                </select>
                <p className="text-xs text-muted-foreground">凭据未显式指定 endpoint 时使用此默认值</p>
              </div>
              <div className="flex items-center justify-between">
                <div className="space-y-1">
                  <label className="text-sm font-medium">凭据冷却机制</label>
                  <p className="text-xs text-muted-foreground">禁用后 429 限流不会触发冷却，仍会尝试故障转移</p>
                </div>
                <Switch checked={enableCredentialCooldown} onCheckedChange={setEnableCredentialCooldown} disabled={isPending} />
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

              {/* 工具截断 */}
              <p className="text-xs font-medium text-muted-foreground pt-2">工具截断</p>
              {numInput('gcTrMaxChars', 'tool_result 截断阈值（字符）', cToolResultMaxChars, setCToolResultMaxChars)}
              <div className="grid grid-cols-2 gap-2">
                {numInput('gcTrHead', 'tool_result 保留头部行数', cToolResultHeadLines, setCToolResultHeadLines)}
                {numInput('gcTrTail', 'tool_result 保留尾部行数', cToolResultTailLines, setCToolResultTailLines)}
              </div>
              {numInput('gcTuMaxChars', 'tool_use input 截断阈值（字符）', cToolUseInputMaxChars, setCToolUseInputMaxChars)}
              {numInput('gcTdMaxChars', '工具描述截断阈值（字符）', cToolDescMaxChars, setCToolDescMaxChars)}
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">工具定义压缩</label>
                <Switch checked={cToolDefCompression} onCheckedChange={setCToolDefCompression} disabled={isPending} aria-label="工具定义压缩" />
              </div>
              {numInput('gcTdSizeThreshold', '工具定义压缩阈值（字符）', cToolDefSizeThreshold, setCToolDefSizeThreshold, '超过此大小的工具定义才压缩')}
              {numInput('gcTdMinDescChars', '工具定义最小描述字符数', cToolDefMinDescChars, setCToolDefMinDescChars)}
              {numInput('gcTnMaxChars', '工具名称最大字符数', cToolNameMaxChars, setCToolNameMaxChars, '0 = 不限')}

              {/* 图片处理 */}
              <p className="text-xs font-medium text-muted-foreground pt-2">图片处理</p>
              {numInput('gcImgLongEdge', '图片最大长边（px）', cImageMaxLongEdge, setCImageMaxLongEdge)}
              <div className="grid grid-cols-2 gap-2">
                {numInput('gcImgPixelsSingle', '单图最大像素', cImageMaxPixelsSingle, setCImageMaxPixelsSingle)}
                {numInput('gcImgPixelsMulti', '多图最大像素', cImageMaxPixelsMulti, setCImageMaxPixelsMulti)}
              </div>
              {numInput('gcImgMultiThreshold', '多图阈值（张）', cImageMultiThreshold, setCImageMultiThreshold, '超过此数量按多图限制')}

              {/* 历史与请求体 */}
              <p className="text-xs font-medium text-muted-foreground pt-2">历史与请求体</p>
              <div className="grid grid-cols-2 gap-2">
                {numInput('gcMaxTurns', '历史最大轮数', cMaxHistoryTurns, setCMaxHistoryTurns, '0 = 不限')}
                {numInput('gcMaxChars', '历史最大字符数', cMaxHistoryChars, setCMaxHistoryChars, '0 = 不限')}
              </div>
              {numInput('gcMaxBody', '请求体大小上限（字节）', cMaxRequestBodyBytes, setCMaxRequestBodyBytes, '超过此大小触发自适应压缩，0 = 不限')}

              {/* 自适应压缩 */}
              <p className="text-xs font-medium text-muted-foreground pt-2">自适应压缩</p>
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">启用自适应压缩</label>
                <Switch checked={cAdaptive} onCheckedChange={setCAdaptive} disabled={isPending} aria-label="启用自适应压缩" />
              </div>
              {numInput('gcAdaptiveMaxIters', '最大迭代次数', cAdaptiveMaxIters, setCAdaptiveMaxIters)}
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">自适应 tool_result 压缩</label>
                <Switch checked={cAdaptiveToolResult} onCheckedChange={setCAdaptiveToolResult} disabled={isPending} aria-label="自适应 tool_result 压缩" />
              </div>
              {numInput('gcAdaptiveMinTrChars', '自适应 tool_result 最小字符数', cAdaptiveMinToolResultChars, setCAdaptiveMinToolResultChars)}
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">自适应 tool_use input 压缩</label>
                <Switch checked={cAdaptiveToolUseInput} onCheckedChange={setCAdaptiveToolUseInput} disabled={isPending} aria-label="自适应 tool_use input 压缩" />
              </div>
              {numInput('gcAdaptiveMinTuChars', '自适应 tool_use input 最小字符数', cAdaptiveMinToolUseInputChars, setCAdaptiveMinToolUseInputChars)}
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">自适应历史图片移除</label>
                <Switch checked={cAdaptiveHistoryImage} onCheckedChange={setCAdaptiveHistoryImage} disabled={isPending} aria-label="自适应历史图片移除" />
              </div>
              <div className="flex items-center justify-between">
                <label className="text-sm font-medium">自适应历史移除</label>
                <Switch checked={cAdaptiveHistoryRemoval} onCheckedChange={setCAdaptiveHistoryRemoval} disabled={isPending} aria-label="自适应历史移除" />
              </div>
              {numInput('gcAdaptivePreserve', '自适应保留最近消息数', cAdaptiveHistoryPreserve, setCAdaptiveHistoryPreserve, '历史移除时保留的最近消息数')}
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
