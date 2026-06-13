import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import {
  getCredentials,
  deleteCredential,
  setCredentialDisabled,
  setCredentialPriority,
  setCredentialRegion,
  setCredentialEndpoint,
  resetCredentialFailure,
  forceRefreshToken,
  getCredentialBalance,
  getCachedBalances,
  addCredential,
  importTokenJson,
  getProxyConfig,
  updateProxyConfig,
  getGlobalConfig,
  updateGlobalConfig,
} from '@/api/credentials'
import type { AddCredentialRequest, ImportTokenJsonRequest, UpdateGlobalConfigRequest } from '@/types/api'

// 查询凭据列表
export function useCredentials() {
  return useQuery({
    queryKey: ['credentials'],
    queryFn: getCredentials,
    refetchInterval: 30000, // 每 30 秒刷新一次
  })
}

// 强制刷新 Token
export function useForceRefreshToken() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => forceRefreshToken(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
      queryClient.invalidateQueries({ queryKey: ['cached-balances'] })
    },
  })
}

// 查询凭据余额
export function useCredentialBalance(id: number | null) {
  return useQuery({
    queryKey: ['credential-balance', id],
    queryFn: () => getCredentialBalance(id!),
    enabled: id !== null,
    retry: false, // 余额查询失败时不重试（避免重复请求被封禁的账号）
  })
}

// 查询所有凭据的缓存余额（定时轮询，带退避策略）
export function useCachedBalances() {
  return useQuery({
    queryKey: ['cached-balances'],
    queryFn: getCachedBalances,
    refetchInterval: (query) => (query.state.error ? 60000 : 30000),
    refetchIntervalInBackground: false, // 页面不可见时暂停轮询
  })
}

// 删除指定凭据
export function useDeleteCredential() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => deleteCredential(id),
    onSuccess: (_res, id) => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
      queryClient.invalidateQueries({ queryKey: ['credential-balance', id] })
    },
  })
}

// 设置禁用状态
export function useSetDisabled() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, disabled }: { id: number; disabled: boolean }) =>
      setCredentialDisabled(id, disabled),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 设置优先级
export function useSetPriority() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, priority }: { id: number; priority: number }) =>
      setCredentialPriority(id, priority),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 设置 Region
export function useSetRegion() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, region, apiRegion }: { id: number; region: string | null; apiRegion: string | null }) =>
      setCredentialRegion(id, region, apiRegion),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 设置 endpoint
export function useSetEndpoint() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, endpoint }: { id: number; endpoint: string | null }) =>
      setCredentialEndpoint(id, endpoint),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 重置失败计数
export function useResetFailure() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => resetCredentialFailure(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 添加新凭据
export function useAddCredential() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (req: AddCredentialRequest) => addCredential(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 批量导入 token.json
export function useImportTokenJson() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (req: ImportTokenJsonRequest) => importTokenJson(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
      queryClient.invalidateQueries({ queryKey: ['cached-balances'] })
    },
  })
}

// 查询全局代理配置
export function useProxyConfig() {
  return useQuery({
    queryKey: ['proxyConfig'],
    queryFn: getProxyConfig,
  })
}

// 更新全局代理配置
export function useUpdateProxyConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: updateProxyConfig,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['proxyConfig'] })
      toast.success('全局代理配置已更新')
    },
    onError: (error: any) => {
      toast.error(error.response?.data?.error?.message || '更新失败')
    },
  })
}

// 查询全局配置
export function useGlobalConfig() {
  return useQuery({
    queryKey: ['globalConfig'],
    queryFn: getGlobalConfig,
  })
}

// 更新全局配置
export function useUpdateGlobalConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (req: UpdateGlobalConfigRequest) => updateGlobalConfig(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['globalConfig'] })
      queryClient.invalidateQueries({ queryKey: ['proxyConfig'] })
      toast.success('全局配置已更新')
    },
    onError: (error: any) => {
      toast.error(error.response?.data?.error?.message || '更新失败')
    },
  })
}
