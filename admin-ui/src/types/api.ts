// 凭据状态响应
export interface CredentialsStatusResponse {
  total: number
  available: number
  credentials: CredentialStatusItem[]
}

// 单个凭据状态
export interface CredentialStatusItem {
  id: number
  priority: number
  disabled: boolean
  failureCount: number
  refreshFailureCount: number
  disabledReason?: string | null
  expiresAt: string | null
  authMethod: string | null
  hasProfileArn: boolean
  accountEmail: string | null
  email?: string
  refreshTokenHash?: string
  subscriptionTitle?: string | null
  apiKeyHash?: string
  maskedApiKey?: string

  // ===== 统计（可持久化） =====
  callsTotal: number
  callsOk: number
  callsErr: number
  inputTokensTotal: number
  outputTokensTotal: number
  lastCallAt: string | null
  lastSuccessAt: string | null
  lastErrorAt: string | null
  lastError: string | null

  // ===== upstream 字段 =====
  successCount: number
  lastUsedAt: string | null
  hasProxy: boolean
  proxyUrl?: string
  /** 凭据级 Region（用于 Token 刷新） */
  region: string | null
  /** 凭据级 API Region（单独覆盖 API 请求） */
  apiRegion: string | null
  /** 凭据显式配置的 endpoint，null 表示回退默认值 */
  endpoint?: string | null
  /** 最终生效的 endpoint */
  effectiveEndpoint: string
}

// 余额响应
export interface BalanceResponse {
  id: number
  subscriptionTitle: string | null
  currentUsage: number
  usageLimit: number
  remaining: number
  usagePercentage: number
  nextResetAt: number | null
  overageEnabled?: boolean | null
  overageStatus?: string | null
}

export interface SetOverageRequest {
  overageEnabled: boolean
}

export interface OverageResponse {
  success: boolean
  message: string
  id: number
  overageEnabled: boolean
  overageStatus?: string | null
}

// 缓存余额信息
export interface CachedBalanceInfo {
  id: number
  currentUsage?: number
  remaining: number
  usageLimit: number
  usagePercentage: number
  subscriptionTitle: string | null
  overageEnabled?: boolean | null
  overageStatus?: string | null
  cachedAt: number // Unix 毫秒时间戳
  ttlSecs: number
}

// 缓存余额响应
export interface CachedBalancesResponse {
  balances: CachedBalanceInfo[]
}

// 成功响应
export interface SuccessResponse {
  success: boolean
  message: string
}

// 错误响应
export interface AdminErrorResponse {
  error: {
    type: string
    message: string
  }
}

// 请求类型
export interface SetDisabledRequest {
  disabled: boolean
}

export interface SetPriorityRequest {
  priority: number
}

export interface SetEndpointRequest {
  endpoint: string | null
}

// 添加凭据请求
export interface AddCredentialRequest {
  refreshToken?: string
  kiroApiKey?: string
  authMethod?: 'social' | 'idc' | 'api_key'
  clientId?: string
  clientSecret?: string
  priority?: number
  /** Region（用于 Token 刷新及默认 API 请求），可被 apiRegion 单独覆盖 */
  region?: string
  /** 单独覆盖 API 请求使用的 region */
  apiRegion?: string
  machineId?: string
  endpoint?: string
  proxyUrl?: string
  proxyUsername?: string
  proxyPassword?: string
}

// 添加凭据响应
export interface AddCredentialResponse {
  success: boolean
  message: string
  credentialId: number
  email?: string
}

// ============ 批量导入 token.json ============

// 官方 token.json 格式（用于解析导入）
export interface TokenJsonItem {
  provider?: string
  refreshToken?: string
  clientId?: string
  clientSecret?: string
  authMethod?: string
  priority?: number
  region?: string
  machineId?: string
}

// 批量导入请求
export interface ImportTokenJsonRequest {
  dryRun?: boolean
  items: TokenJsonItem | TokenJsonItem[]
}

// 导入动作
export type ImportAction = 'added' | 'skipped' | 'invalid'

// 单项导入结果
export interface ImportItemResult {
  index: number
  fingerprint: string
  action: ImportAction
  reason?: string
  credentialId?: number
}

// 导入汇总
export interface ImportSummary {
  parsed: number
  added: number
  skipped: number
  invalid: number
}

// 批量导入响应
export interface ImportTokenJsonResponse {
  summary: ImportSummary
  items: ImportItemResult[]
}

// ============ 全局代理配置 ============

export interface ProxyConfigResponse {
  proxyUrl: string | null
  hasCredentials: boolean
}

export interface UpdateProxyConfigRequest {
  proxyUrl?: string | null
  proxyUsername?: string | null
  proxyPassword?: string | null
}

// ============ 全局配置 ============

export interface CompressionConfigResponse {
  enabled: boolean
  whitespaceCompression: boolean
  thinkingStrategy: string
  toolDescriptionMaxChars: number
  toolDefinitionCompression: boolean
  toolDefinitionMinDescriptionChars: number
  toolNameMaxChars: number
  maxRequestBodyBytes: number
  adaptiveCompression: boolean
  adaptiveCompressionMaxIters: number
}

export interface GlobalConfigResponse {
  region: string
  credentialRpm: number | null
  promptCacheTtlSeconds: number
  promptCacheMode: 'upstream' | 'simulated' | 'off'
  defaultEndpoint: string
  enableCredentialCooldown: boolean
  enableRateLimit: boolean
  enableStickyRouting: boolean
  autoDisableInsufficientBalance: boolean
  autoDisableRefreshFailure: boolean
  autoDisableOnForbidden: boolean
  compression: CompressionConfigResponse
  rewriter: RewriterConfigResponse
}

export interface RewriterConfigResponse {
  enabled: boolean
  keywords: string[]
}

export interface UpdateCompressionConfigRequest {
  enabled?: boolean
  whitespaceCompression?: boolean
  thinkingStrategy?: string
  toolDescriptionMaxChars?: number
  toolDefinitionCompression?: boolean
  toolDefinitionMinDescriptionChars?: number
  toolNameMaxChars?: number
  maxRequestBodyBytes?: number
  adaptiveCompression?: boolean
  adaptiveCompressionMaxIters?: number
}

export interface UpdateGlobalConfigRequest {
  region?: string
  credentialRpm?: number | null
  promptCacheTtlSeconds?: number
  promptCacheMode?: 'upstream' | 'simulated' | 'off'
  defaultEndpoint?: string
  enableCredentialCooldown?: boolean
  enableRateLimit?: boolean
  enableStickyRouting?: boolean
  autoDisableInsufficientBalance?: boolean
  autoDisableRefreshFailure?: boolean
  autoDisableOnForbidden?: boolean
  compression?: UpdateCompressionConfigRequest
  rewriter?: UpdateRewriterConfigRequest
}

export interface UpdateRewriterConfigRequest {
  enabled?: boolean
  keywords?: string[]
}
