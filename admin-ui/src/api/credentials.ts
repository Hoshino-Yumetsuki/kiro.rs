import axios from 'axios'
import { storage } from '@/lib/storage'
import type {
  CredentialsStatusResponse,
  BalanceResponse,
  CachedBalancesResponse,
  SuccessResponse,
  OverageResponse,
  SetDisabledRequest,
  SetOverageRequest,
  SetPriorityRequest,
  SetEndpointRequest,
  AddCredentialRequest,
  AddCredentialResponse,

  ImportTokenJsonRequest,
  ImportTokenJsonResponse,
  ProxyConfigResponse,
  UpdateProxyConfigRequest,
  GlobalConfigResponse,
  UpdateGlobalConfigRequest,
} from '@/types/api'

// 创建 axios 实例
const api = axios.create({
  baseURL: '/api/admin',
  headers: {
    'Content-Type': 'application/json',
  },
})

// 请求拦截器添加 API Key
api.interceptors.request.use((config) => {
  const apiKey = storage.getApiKey()
  if (apiKey) {
    config.headers['x-api-key'] = apiKey
  }
  return config
})

// 获取所有凭据状态
export async function getCredentials(): Promise<CredentialsStatusResponse> {
  const { data } = await api.get<CredentialsStatusResponse>('/credentials')
  return data
}

// 设置凭据禁用状态
export async function setCredentialDisabled(
  id: number,
  disabled: boolean,
  signal?: AbortSignal
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(
    `/credentials/${id}/disabled`,
    { disabled } as SetDisabledRequest,
    { signal }
  )
  return data
}

// 设置凭据优先级
export async function setCredentialPriority(
  id: number,
  priority: number
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(
    `/credentials/${id}/priority`,
    { priority } as SetPriorityRequest
  )
  return data
}

// 重置失败计数
export async function resetCredentialFailure(
  id: number,
  signal?: AbortSignal
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(`/credentials/${id}/reset`, undefined, { signal })
  return data
}

// 设置凭据 Region
export async function setCredentialRegion(
  id: number,
  region: string | null,
  apiRegion: string | null
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(`/credentials/${id}/region`, {
    region: region || null,
    apiRegion: apiRegion || null,
  })
  return data
}

// 设置凭据 endpoint
export async function setCredentialEndpoint(
  id: number,
  endpoint: string | null
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(
    `/credentials/${id}/endpoint`,
    { endpoint } as SetEndpointRequest
  )
  return data
}

// 强制刷新 Token
export async function forceRefreshToken(id: number, signal?: AbortSignal): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(`/credentials/${id}/refresh`, undefined, { signal })
  return data
}

// 设置凭据 overage 状态
export async function setCredentialOverage(
  id: number,
  overageEnabled: boolean,
  signal?: AbortSignal
): Promise<OverageResponse> {
  const { data } = await api.post<OverageResponse>(
    `/credentials/${id}/overage`,
    { overageEnabled } as SetOverageRequest,
    { signal }
  )
  return data
}

// 获取凭据余额
export async function getCredentialBalance(id: number, signal?: AbortSignal): Promise<BalanceResponse> {
  const { data } = await api.get<BalanceResponse>(`/credentials/${id}/balance`, { signal })
  return data
}

// 获取所有凭据的缓存余额
export async function getCachedBalances(): Promise<CachedBalancesResponse> {
  const { data } = await api.get<CachedBalancesResponse>('/credentials/balances/cached')
  return data
}

// 添加新凭据
export async function addCredential(
  req: AddCredentialRequest,
  signal?: AbortSignal
): Promise<AddCredentialResponse> {
  const { data } = await api.post<AddCredentialResponse>('/credentials', req, { signal })
  return data
}

// 删除凭据
export async function deleteCredential(id: number, signal?: AbortSignal): Promise<SuccessResponse> {
  const { data } = await api.delete<SuccessResponse>(`/credentials/${id}`, { signal })
  return data
}

// 批量导入 token.json
export async function importTokenJson(
  req: ImportTokenJsonRequest
): Promise<ImportTokenJsonResponse> {
  const { data } = await api.post<ImportTokenJsonResponse>(
    '/credentials/import-token-json',
    req
  )
  return data
}

// 获取全局代理配置
export async function getProxyConfig(): Promise<ProxyConfigResponse> {
  const { data } = await api.get<ProxyConfigResponse>('/proxy')
  return data
}

// 更新全局代理配置
export async function updateProxyConfig(req: UpdateProxyConfigRequest): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>('/proxy', req)
  return data
}

// 获取全局配置
export async function getGlobalConfig(): Promise<GlobalConfigResponse> {
  const { data } = await api.get<GlobalConfigResponse>('/config/global')
  return data
}

// 更新全局配置
export async function updateGlobalConfig(req: UpdateGlobalConfigRequest): Promise<SuccessResponse> {
  const { data } = await api.put<SuccessResponse>('/config/global', req)
  return data
}
