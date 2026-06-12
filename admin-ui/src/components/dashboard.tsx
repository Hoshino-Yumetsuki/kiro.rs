import { useState, useEffect, useRef, useMemo, useCallback, type MouseEvent } from 'react'
import PQueue from 'p-queue'
import { RefreshCw, LogOut, Moon, Sun, Server, Plus, Upload, Trash2, RotateCcw, CheckCircle2, ArrowUp, ArrowDown, Wallet, Eraser, Settings } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { useVirtualizer } from '@tanstack/react-virtual'
import { toast } from 'sonner'
import { storage } from '@/lib/storage'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { CredentialCard } from '@/components/credential-card'
import { BalanceDialog } from '@/components/balance-dialog'
import { AddCredentialDialog } from '@/components/add-credential-dialog'
import { ImportTokenJsonDialog } from '@/components/import-token-json-dialog'
import { BatchVerifyDialog, type VerifyResult } from '@/components/batch-verify-dialog'
import { GlobalConfigDialog } from '@/components/global-config-dialog'
import { useCredentials, useCachedBalances, useProxyConfig, useGlobalConfig } from '@/hooks/use-credentials'
import { deleteCredential, forceRefreshToken, getCredentialBalance, resetCredentialFailure } from '@/api/credentials'
import { formatKiroUsageWithUsd } from '@/lib/format'
import { extractErrorMessage } from '@/lib/utils'
import { Badge } from '@/components/ui/badge'
import type { BalanceResponse } from '@/types/api'

type SortField = 'default' | 'id' | 'balance'
type SortOrder = 'asc' | 'desc'

interface DashboardProps {
  onLogout: () => void
}

export function Dashboard({ onLogout }: DashboardProps) {
  const [selectedCredentialId, setSelectedCredentialId] = useState<number | null>(null)
  const [balanceDialogOpen, setBalanceDialogOpen] = useState(false)
  const [forceRefreshBalance, setForceRefreshBalance] = useState(false)
  const [addDialogOpen, setAddDialogOpen] = useState(false)
  const [importDialogOpen, setImportDialogOpen] = useState(false)
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [verifyDialogOpen, setVerifyDialogOpen] = useState(false)
  const [globalConfigDialogOpen, setGlobalConfigDialogOpen] = useState(false)
  const [verifying, setVerifying] = useState(false)
  const [verifyProgress, setVerifyProgress] = useState({ current: 0, total: 0 })
  const [verifyResults, setVerifyResults] = useState<Map<number, VerifyResult>>(new Map())
  const [balanceMap, setBalanceMap] = useState<Map<number, BalanceResponse>>(new Map())
  const [loadingBalanceIds, setLoadingBalanceIds] = useState<Set<number>>(new Set())
  const balanceMapRef = useRef(balanceMap)
  const loadingBalanceIdsRef = useRef(loadingBalanceIds)
  const [queryingInfo, setQueryingInfo] = useState(false)
  const [queryInfoProgress, setQueryInfoProgress] = useState({ current: 0, total: 0 })
  const [batchDeleteDialogOpen, setBatchDeleteDialogOpen] = useState(false)
  const [clearAllDialogOpen, setClearAllDialogOpen] = useState(false)
  const [pendingBatchDeleteIds, setPendingBatchDeleteIds] = useState<number[]>([])
  const [pendingClearAllCount, setPendingClearAllCount] = useState(0)
  const activeBatchCancelRef = useRef<(() => void) | null>(null)
  const cancelVerifyRef = useRef<(() => void) | null>(null)
  const cancelQueryInfoRef = useRef<(() => void) | null>(null)
  const autoHydratedBalanceIdsRef = useRef<Set<number>>(new Set())
  const parentRef = useRef<HTMLDivElement>(null)
  const lastSelectedIdRef = useRef<number | null>(null)
  const selectAllCheckboxRef = useRef<HTMLInputElement>(null)
  const [sortField, setSortField] = useState<SortField>('default')
  const [sortOrder, setSortOrder] = useState<SortOrder>('asc')
  const [columnCount, setColumnCount] = useState(1)
  const [darkMode, setDarkMode] = useState(() => {
    if (typeof window !== 'undefined') {
      const saved = localStorage.getItem('kiro-dark-mode')
      if (saved !== null) {
        const isDark = saved === 'true'
        document.documentElement.classList.toggle('dark', isDark)
        return isDark
      }
      return document.documentElement.classList.contains('dark')
    }
    return false
  })

  const queryClient = useQueryClient()
  const { data, isLoading, error, refetch } = useCredentials()
  const { data: cachedBalancesData } = useCachedBalances()
  useProxyConfig()
  useGlobalConfig()

  // 构建 id -> cachedBalance 的映射
  const cachedBalanceMap = new Map(
    cachedBalancesData?.balances.map((b) => [b.id, b]) ?? []
  )
  const cachedBalances = cachedBalancesData?.balances

  useEffect(() => {
    balanceMapRef.current = balanceMap
  }, [balanceMap])

  useEffect(() => {
    loadingBalanceIdsRef.current = loadingBalanceIds
  }, [loadingBalanceIds])

  // 排序后的凭据列表
  const sortedCredentials = useMemo(() => {
    const credentials = data?.credentials || []
    if (sortField === 'default') return credentials

    return [...credentials].sort((a, b) => {
      let cmp = 0
      if (sortField === 'id') {
        cmp = a.id - b.id
      } else if (sortField === 'balance') {
        const balA = cachedBalanceMap.get(a.id)?.remaining ?? -Infinity
        const balB = cachedBalanceMap.get(b.id)?.remaining ?? -Infinity
        cmp = balA - balB
      }
      return sortOrder === 'asc' ? cmp : -cmp
    })
  }, [data?.credentials, sortField, sortOrder, cachedBalanceMap])

  const credentialRows = useMemo(() => {
    const rows: typeof sortedCredentials[] = []
    for (let i = 0; i < sortedCredentials.length; i += columnCount) {
      rows.push(sortedCredentials.slice(i, i + columnCount))
    }
    return rows
  }, [sortedCredentials, columnCount])

  const rowVirtualizer = useVirtualizer({
    count: credentialRows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 260,
    overscan: 3,
  })

  const disabledCredentialCount = data?.credentials.filter(credential => credential.disabled).length || 0
  const selectedDisabledCount = Array.from(selectedIds).filter(id => {
    const credential = data?.credentials.find(c => c.id === id)
    return Boolean(credential?.disabled)
  }).length
  const allVisibleSelected = sortedCredentials.length > 0 && sortedCredentials.every(credential => selectedIds.has(credential.id))
  const someVisibleSelected = sortedCredentials.some(credential => selectedIds.has(credential.id))

  useEffect(() => {
    if (selectAllCheckboxRef.current) {
      selectAllCheckboxRef.current.indeterminate = someVisibleSelected && !allVisibleSelected
    }
  }, [allVisibleSelected, someVisibleSelected])

  useEffect(() => {
    if (typeof window === 'undefined') return

    const updateColumnCount = () => {
      if (window.matchMedia('(min-width: 1024px)').matches) {
        setColumnCount(3)
      } else if (window.matchMedia('(min-width: 768px)').matches) {
        setColumnCount(2)
      } else {
        setColumnCount(1)
      }
    }

    updateColumnCount()
    window.addEventListener('resize', updateColumnCount)
    return () => window.removeEventListener('resize', updateColumnCount)
  }, [])

  // 只保留当前仍存在的凭据缓存，避免删除后残留旧数据
  useEffect(() => {
    if (!data?.credentials) {
      setBalanceMap(new Map())
      setLoadingBalanceIds(new Set())
      autoHydratedBalanceIdsRef.current.clear()
      return
    }

    const validIds = new Set(data.credentials.map(credential => credential.id))

    setBalanceMap(prev => {
      const next = new Map<number, BalanceResponse>()
      prev.forEach((value, id) => {
        if (validIds.has(id)) {
          next.set(id, value)
        }
      })
      return next.size === prev.size ? prev : next
    })

    setLoadingBalanceIds(prev => {
      if (prev.size === 0) {
        return prev
      }
      const next = new Set<number>()
      prev.forEach(id => {
        if (validIds.has(id)) {
          next.add(id)
        }
      })
      return next.size === prev.size ? prev : next
    })
  }, [data?.credentials])

  useEffect(() => {
    if (!data?.credentials || !cachedBalances?.length) {
      return
    }

    const validIds = new Set(data.credentials.map(credential => credential.id))
    const idsToHydrate = cachedBalances
      .filter(balance => (
        validIds.has(balance.id)
        && balance.usageLimit > 0
        && balance.remaining <= 0
        && balance.currentUsage === undefined
        && !balanceMapRef.current.has(balance.id)
        && !loadingBalanceIdsRef.current.has(balance.id)
        && !autoHydratedBalanceIdsRef.current.has(balance.id)
      ))
      .map(balance => balance.id)

    if (idsToHydrate.length === 0) {
      return
    }

    idsToHydrate.forEach(id => autoHydratedBalanceIdsRef.current.add(id))

    setLoadingBalanceIds(prev => {
      const next = new Set(prev)
      idsToHydrate.forEach(id => next.add(id))
      return next
    })

    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 3 })

    idsToHydrate.forEach(id => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        try {
          const balance = await getCredentialBalance(id, controller.signal)
          if (controller.signal.aborted) return

          setBalanceMap(prev => {
            const next = new Map(prev)
            next.set(id, balance)
            return next
          })
        } catch (error) {
          if (controller.signal.aborted) return
          autoHydratedBalanceIdsRef.current.delete(id)
          console.warn(`自动补全凭据 #${id} 余额失败`, error)
        } finally {
          if (controller.signal.aborted) return
          setLoadingBalanceIds(prev => {
            const next = new Set(prev)
            next.delete(id)
            return next
          })
        }
      })
    })

    return () => {
      queue.clear()
      controller.abort()
    }
  }, [cachedBalances, data?.credentials])

  const toggleDarkMode = () => {
    const newMode = !darkMode
    setDarkMode(newMode)
    document.documentElement.classList.toggle('dark', newMode)
    localStorage.setItem('kiro-dark-mode', String(newMode))
  }

  const handleViewBalance = (id: number, forceRefresh: boolean) => {
    setSelectedCredentialId(id)
    setForceRefreshBalance(forceRefresh)
    if (forceRefresh) {
      // 清除该凭据的余额缓存，强制重新获取
      queryClient.invalidateQueries({ queryKey: ['credential-balance', id] })
    }
    setBalanceDialogOpen(true)
  }

  const handleBalanceLoaded = useCallback((id: number, balance: BalanceResponse) => {
    setBalanceMap(prev => {
      if (prev.get(id) === balance) {
        return prev
      }
      const next = new Map(prev)
      next.set(id, balance)
      return next
    })
  }, [])

  const handleRefresh = () => {
    refetch()
    toast.success('已刷新凭据列表')
  }

  const handleLogout = () => {
    storage.removeApiKey()
    queryClient.clear()
    onLogout()
  }

  // 排序切换
  const handleSortChange = (field: SortField) => {
    if (field === sortField) {
      // 同一字段：切换方向
      setSortOrder(prev => prev === 'asc' ? 'desc' : 'asc')
    } else {
      setSortField(field)
      setSortOrder(field === 'balance' ? 'desc' : 'asc')
    }
    parentRef.current?.scrollTo({ top: 0 })
  }

  // 选择管理
  const handleToggleSelect = useCallback((id: number, event?: MouseEvent) => {
    if (event?.shiftKey && lastSelectedIdRef.current !== null) {
      const allIds = sortedCredentials.map(credential => credential.id)
      const startIndex = allIds.indexOf(lastSelectedIdRef.current)
      const endIndex = allIds.indexOf(id)

      if (startIndex !== -1 && endIndex !== -1) {
        const [from, to] = startIndex < endIndex ? [startIndex, endIndex] : [endIndex, startIndex]
        setSelectedIds(prev => {
          const next = new Set(prev)
          for (let i = from; i <= to; i++) {
            next.add(allIds[i])
          }
          return next
        })
        lastSelectedIdRef.current = id
        return
      }
    }

    setSelectedIds(prev => {
      const next = new Set(prev)
      if (next.has(id)) {
        next.delete(id)
      } else {
        next.add(id)
      }
      return next
    })
    lastSelectedIdRef.current = id
  }, [sortedCredentials])

  const handleSelectAllVisible = useCallback(() => {
    const visibleIds = sortedCredentials.map(credential => credential.id)
    const allSelected = visibleIds.length > 0 && visibleIds.every(id => selectedIds.has(id))

    setSelectedIds(prev => {
      const next = new Set(prev)
      if (allSelected) {
        visibleIds.forEach(id => next.delete(id))
      } else {
        visibleIds.forEach(id => next.add(id))
      }
      return next
    })
  }, [selectedIds, sortedCredentials])

  const deselectAll = () => {
    setSelectedIds(new Set())
    lastSelectedIdRef.current = null
  }

  // 批量删除（仅删除已禁用项）
  const handleBatchDelete = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要删除的凭据')
      return
    }

    const disabledIds = Array.from(selectedIds).filter(id => {
      const credential = data?.credentials.find(c => c.id === id)
      return Boolean(credential?.disabled)
    })

    if (disabledIds.length === 0) {
      toast.error('选中的凭据中没有已禁用项')
      return
    }

    setPendingBatchDeleteIds(disabledIds)
    setBatchDeleteDialogOpen(true)
  }

  const confirmBatchDelete = async () => {
    const disabledIds = pendingBatchDeleteIds
    const skippedCount = selectedIds.size - disabledIds.length
    setBatchDeleteDialogOpen(false)

    let successCount = 0
    let failCount = 0
    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 5 })
    activeBatchCancelRef.current = () => {
      queue.clear()
      controller.abort()
    }

    disabledIds.forEach(id => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        try {
          await deleteCredential(id, controller.signal)
          if (!controller.signal.aborted) {
            successCount++
          }
        } catch (error) {
          if (!controller.signal.aborted) {
            failCount++
          }
        }
      })
    })

    await queue.onIdle()
    activeBatchCancelRef.current = null
    queryClient.invalidateQueries({ queryKey: ['credentials'] })

    const skippedResultText = skippedCount > 0 ? `，已跳过 ${skippedCount} 个未禁用凭据` : ''

    if (failCount === 0) {
      toast.success(`成功删除 ${successCount} 个已禁用凭据${skippedResultText}`)
    } else {
      toast.warning(`删除已禁用凭据：成功 ${successCount} 个，失败 ${failCount} 个${skippedResultText}`)
    }

    deselectAll()
  }

  // 批量恢复异常
  const handleBatchResetFailure = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要恢复的凭据')
      return
    }

    const failedIds = Array.from(selectedIds).filter(id => {
      const cred = data?.credentials.find(c => c.id === id)
      return cred && (cred.failureCount > 0 || cred.refreshFailureCount > 0)
    })

    if (failedIds.length === 0) {
      toast.error('选中的凭据中没有失败的凭据')
      return
    }

    let successCount = 0
    let failCount = 0
    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 5 })
    activeBatchCancelRef.current = () => {
      queue.clear()
      controller.abort()
    }

    failedIds.forEach(id => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        try {
          await resetCredentialFailure(id, controller.signal)
          if (!controller.signal.aborted) {
            successCount++
          }
        } catch (error) {
          if (!controller.signal.aborted) {
            failCount++
          }
        }
      })
    })

    await queue.onIdle()
    activeBatchCancelRef.current = null
    queryClient.invalidateQueries({ queryKey: ['credentials'] })

    if (failCount === 0) {
      toast.success(`成功恢复 ${successCount} 个凭据`)
    } else {
      toast.warning(`成功 ${successCount} 个，失败 ${failCount} 个`)
    }

    deselectAll()
  }

  const handleBatchForceRefresh = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要刷新的凭据')
      return
    }

    const ids = Array.from(selectedIds)
    let successCount = 0
    let failCount = 0
    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 5 })
    activeBatchCancelRef.current = () => {
      queue.clear()
      controller.abort()
    }

    ids.forEach(id => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        try {
          await forceRefreshToken(id, controller.signal)
          if (!controller.signal.aborted) {
            successCount++
          }
        } catch {
          if (!controller.signal.aborted) {
            failCount++
          }
        }
      })
    })

    await queue.onIdle()
    activeBatchCancelRef.current = null
    queryClient.invalidateQueries({ queryKey: ['credentials'] })
    queryClient.invalidateQueries({ queryKey: ['cached-balances'] })

    if (failCount === 0) {
      toast.success(`成功刷新 ${successCount} 个凭据`)
    } else {
      toast.warning(`刷新完成：成功 ${successCount} 个，失败 ${failCount} 个`)
    }
  }

  // 一键清除所有已禁用凭据
  const handleClearAll = async () => {
    if (!data?.credentials || data.credentials.length === 0) {
      toast.error('没有可清除的凭据')
      return
    }

    const disabledCredentials = data.credentials.filter(credential => credential.disabled)

    if (disabledCredentials.length === 0) {
      toast.error('没有可清除的已禁用凭据')
      return
    }

    setPendingClearAllCount(disabledCredentials.length)
    setClearAllDialogOpen(true)
  }

  const confirmClearAll = async () => {
    const disabledCredentials = data?.credentials.filter(credential => credential.disabled) ?? []
    setClearAllDialogOpen(false)

    let successCount = 0
    let failCount = 0
    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 5 })
    activeBatchCancelRef.current = () => {
      queue.clear()
      controller.abort()
    }

    disabledCredentials.forEach(credential => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        try {
          await deleteCredential(credential.id, controller.signal)
          if (!controller.signal.aborted) {
            successCount++
          }
        } catch (error) {
          if (!controller.signal.aborted) {
            failCount++
          }
        }
      })
    })

    await queue.onIdle()
    activeBatchCancelRef.current = null
    queryClient.invalidateQueries({ queryKey: ['credentials'] })

    if (failCount === 0) {
      toast.success(`成功清除所有 ${successCount} 个已禁用凭据`)
    } else {
      toast.warning(`清除已禁用凭据：成功 ${successCount} 个，失败 ${failCount} 个`)
    }

    deselectAll()
  }

  // 查询当前视图凭据信息（逐个查询，避免瞬时并发）
  const handleQueryCurrentPageInfo = async () => {
    if (sortedCredentials.length === 0) {
      toast.error('当前视图没有可查询的凭据')
      return
    }

    const ids = sortedCredentials
      .filter(credential => !credential.disabled)
      .map(credential => credential.id)

    if (ids.length === 0) {
      toast.error('当前视图没有可查询的启用凭据')
      return
    }

    setQueryingInfo(true)
    setQueryInfoProgress({ current: 0, total: ids.length })

    let successCount = 0
    let failCount = 0

    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 3 })
    cancelQueryInfoRef.current = () => {
      queue.clear()
      controller.abort()
    }
    let completedCount = 0

    ids.forEach(id => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        setLoadingBalanceIds(prev => {
          const next = new Set(prev)
          next.add(id)
          return next
        })

        try {
          const balance = await getCredentialBalance(id, controller.signal)
          if (controller.signal.aborted) return

          successCount++
          setBalanceMap(prev => {
            const next = new Map(prev)
            next.set(id, balance)
            return next
          })
        } catch (error) {
          if (!controller.signal.aborted) {
            failCount++
          }
        } finally {
          setLoadingBalanceIds(prev => {
            const next = new Set(prev)
            next.delete(id)
            return next
          })

          if (!controller.signal.aborted) {
            completedCount++
            setQueryInfoProgress({ current: completedCount, total: ids.length })
          }
        }
      })
    })

    await queue.onIdle()
    cancelQueryInfoRef.current = null

    setQueryingInfo(false)

    if (controller.signal.aborted) {
      return
    }

    if (failCount === 0) {
      toast.success(`查询完成：成功 ${successCount}/${ids.length}`)
    } else {
      toast.warning(`查询完成：成功 ${successCount} 个，失败 ${failCount} 个`)
    }
  }

  // 批量验活
  const handleBatchVerify = async () => {
    if (selectedIds.size === 0) {
      toast.error('请先选择要验活的凭据')
      return
    }

    // 初始化状态
    setVerifying(true)
    const ids = Array.from(selectedIds)
    setVerifyProgress({ current: 0, total: ids.length })
    const controller = new AbortController()
    const queue = new PQueue({ concurrency: 5 })
    cancelVerifyRef.current = () => {
      queue.clear()
      controller.abort()
    }

    let successCount = 0

    // 初始化结果，所有凭据状态为 pending
    const initialResults = new Map<number, VerifyResult>()
    ids.forEach(id => {
      initialResults.set(id, { id, status: 'pending' })
    })
    setVerifyResults(initialResults)
    setVerifyDialogOpen(true)

    setVerifyResults(prev => {
      const newResults = new Map(prev)
      ids.forEach(id => {
        newResults.set(id, { id, status: 'verifying' })
      })
      return newResults
    })

    let completedCount = 0

    ids.forEach(id => {
      void queue.add(async () => {
        if (controller.signal.aborted) return

        try {
          const balance = await getCredentialBalance(id, controller.signal)
          if (controller.signal.aborted) return

          successCount++

          setVerifyResults(prev => {
            const newResults = new Map(prev)
            newResults.set(id, {
              id,
              status: 'success',
              usage: formatKiroUsageWithUsd(balance.currentUsage, balance.usageLimit)
            })
            return newResults
          })
        } catch (error) {
          if (controller.signal.aborted) return

          setVerifyResults(prev => {
            const newResults = new Map(prev)
            newResults.set(id, {
              id,
              status: 'failed',
              error: extractErrorMessage(error)
            })
            return newResults
          })
        } finally {
          completedCount++
          setVerifyProgress({ current: completedCount, total: ids.length })
        }
      })
    })

    await queue.onIdle()

    setVerifying(false)
    cancelVerifyRef.current = null

    if (!controller.signal.aborted) {
      toast.success(`验活完成：成功 ${successCount}/${ids.length}`)
    }
  }

  // 取消验活
  const handleCancelVerify = () => {
    cancelVerifyRef.current?.()
    cancelVerifyRef.current = null
    setVerifying(false)
  }

  if (isLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-background" aria-live="polite" aria-busy="true">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-primary mx-auto mb-4" aria-hidden="true"></div>
          <p className="text-muted-foreground">加载中…</p>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-background p-4">
        <Card className="w-full max-w-md">
          <CardContent className="pt-6 text-center">
            <div className="text-red-500 mb-4">加载失败</div>
            <p className="text-muted-foreground mb-4">{(error as Error).message}</p>
            <div className="space-x-2">
              <Button onClick={() => refetch()}>重试</Button>
              <Button variant="outline" onClick={handleLogout}>重新登录</Button>
            </div>
          </CardContent>
        </Card>
      </div>
    )
  }

  return (
    <div className="min-h-screen bg-background">
      {/* 顶部导航 */}
      <header className="sticky top-0 z-50 w-full border-b bg-background/95 backdrop-blur supports-backdrop-filter:bg-background/60">
        <div className="container mx-auto flex h-14 items-center justify-between px-4 md:px-8">
          <div className="flex items-center gap-2">
            <Server className="h-5 w-5" aria-hidden="true" />
            <span className="font-semibold">Kiro Admin</span>
          </div>
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="icon" onClick={toggleDarkMode} aria-label={darkMode ? '切换到浅色模式' : '切换到深色模式'}>
              {darkMode ? <Sun className="h-5 w-5" aria-hidden="true" /> : <Moon className="h-5 w-5" aria-hidden="true" />}
            </Button>
            <Button variant="ghost" size="icon" onClick={handleRefresh} aria-label="刷新凭据列表">
              <RefreshCw className="h-5 w-5" aria-hidden="true" />
            </Button>
            <Button variant="ghost" size="icon" onClick={handleLogout} aria-label="退出登录">
              <LogOut className="h-5 w-5" aria-hidden="true" />
            </Button>
          </div>
        </div>
      </header>

      {/* 主内容 */}
      <main className="container mx-auto px-4 md:px-8 py-6">
        {/* 摘要栏 */}
        <div className="flex items-center justify-between rounded-lg border bg-card px-4 py-3 mb-6">
          <div className="flex items-center gap-2 text-sm font-medium">
            <span className="text-foreground">{data?.total || 0} 凭据</span>
            <span className="text-muted-foreground">
              ({data?.available || 0} 可用 / {disabledCredentialCount} 已禁用)
            </span>
          </div>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setGlobalConfigDialogOpen(true)}
              aria-label="全局配置"
              title="全局配置"
            >
              <Settings className="h-5 w-5" aria-hidden="true" />
            </Button>
          </div>
        </div>

        {/* 凭据列表 */}
        <div className="space-y-4">
          {/* 工具栏：始终可见行 */}
          <div className="flex items-center justify-between flex-wrap gap-2">
            <div className="flex items-center gap-2">
              <label className="flex items-center gap-2 text-sm text-muted-foreground">
                <input
                  ref={selectAllCheckboxRef}
                  type="checkbox"
                  checked={allVisibleSelected}
                  onChange={handleSelectAllVisible}
                  disabled={sortedCredentials.length === 0}
                  className="h-4 w-4 rounded border-gray-300"
                  aria-label="选择当前视图全部凭据"
                />
                <span>
                  {selectedIds.size > 0 ? `已选 ${selectedIds.size} 项` : '全选'}
                </span>
              </label>
              {/* 排序控件 */}
              <div className="flex items-center gap-1">
                {([
                  ['default', '默认'],
                  ['id', 'ID'],
                  ['balance', '余额'],
                ] as const).map(([field, label]) => {
                  const active = sortField === field
                  return (
                    <Button
                      key={field}
                      size="sm"
                      variant={active ? 'secondary' : 'ghost'}
                      className="h-7 px-2 text-xs"
                      onClick={() => handleSortChange(field)}
                    >
                      {label}
                      {active && field !== 'default' && (
                        sortOrder === 'asc'
                          ? <ArrowUp className="h-3 w-3 ml-0.5" aria-hidden="true" />
                          : <ArrowDown className="h-3 w-3 ml-0.5" aria-hidden="true" />
                      )}
                    </Button>
                  )
                })}
              </div>
            </div>
            <div className="flex items-center gap-2">
              {verifying && !verifyDialogOpen && (
                <Button onClick={() => setVerifyDialogOpen(true)} size="sm" variant="secondary">
                  <CheckCircle2 className="h-4 w-4 mr-2 animate-spin" aria-hidden="true" />
                  验活中… {verifyProgress.current}/{verifyProgress.total}
                </Button>
              )}
              <Button
                onClick={queryingInfo ? () => cancelQueryInfoRef.current?.() : handleQueryCurrentPageInfo}
                size="sm"
                variant="outline"
                disabled={!queryingInfo && (!data?.credentials || data.credentials.length === 0)}
              >
                <Wallet className={`h-4 w-4 mr-2 ${queryingInfo ? 'animate-pulse' : ''}`} aria-hidden="true" />
                {queryingInfo ? `取消查询… ${queryInfoProgress.current}/${queryInfoProgress.total}` : '查询信息'}
              </Button>
              <Button
                onClick={handleClearAll}
                size="sm"
                variant="outline"
                className="text-destructive hover:text-destructive"
                disabled={disabledCredentialCount === 0}
                title={disabledCredentialCount === 0 ? '没有可清除的已禁用凭据' : undefined}
              >
                <Eraser className="h-4 w-4 mr-2" aria-hidden="true" />
                清除已禁用
              </Button>
              <Button variant="outline" onClick={() => setImportDialogOpen(true)} size="sm">
                <Upload className="h-4 w-4 mr-2" aria-hidden="true" />
                导入凭据
              </Button>
              <Button onClick={() => setAddDialogOpen(true)} size="sm">
                <Plus className="h-4 w-4 mr-2" aria-hidden="true" />
                添加凭据
              </Button>
            </div>
          </div>

          {/* 批量操作栏：选中时显示 */}
          {selectedIds.size > 0 && (
            <div className="flex items-center gap-2 rounded-lg border bg-muted/50 px-4 py-2">
              <Badge variant="secondary">已选 {selectedIds.size} 项</Badge>
              <Button onClick={deselectAll} size="sm" variant="ghost" className="h-7 text-xs">
                取消选择
              </Button>
              <div className="ml-auto flex items-center gap-2">
                <Button onClick={handleBatchVerify} size="sm" variant="outline">
                  <CheckCircle2 className="h-4 w-4 mr-2" aria-hidden="true" />
                  批量验活
                </Button>
                <Button onClick={handleBatchForceRefresh} size="sm" variant="outline">
                  <RefreshCw className="h-4 w-4 mr-2" aria-hidden="true" />
                  批量刷新
                </Button>
                <Button onClick={handleBatchResetFailure} size="sm" variant="outline">
                  <RotateCcw className="h-4 w-4 mr-2" aria-hidden="true" />
                  重置失败
                </Button>
                <Button
                  onClick={handleBatchDelete}
                  size="sm"
                  variant="destructive"
                  disabled={selectedDisabledCount === 0}
                  title={selectedDisabledCount === 0 ? '只能删除已禁用凭据' : undefined}
                >
                  <Trash2 className="h-4 w-4 mr-2" aria-hidden="true" />
                  批量删除
                </Button>
              </div>
            </div>
          )}
          {data?.credentials.length === 0 ? (
            <Card>
              <CardContent className="py-12 text-center">
                <Server className="h-12 w-12 mx-auto mb-4 text-muted-foreground/50" aria-hidden="true" />
                <p className="text-lg font-medium mb-1">暂无凭据</p>
                <p className="text-sm text-muted-foreground mb-4">添加凭据以开始使用代理服务</p>
                <Button onClick={() => setAddDialogOpen(true)} size="sm">
                  <Plus className="h-4 w-4 mr-2" aria-hidden="true" />
                  添加凭据
                </Button>
              </CardContent>
            </Card>
          ) : (
            <div ref={parentRef} className="h-[calc(100vh-260px)] overflow-auto pr-1">
              <div
                style={{
                  height: `${rowVirtualizer.getTotalSize()}px`,
                  position: 'relative',
                }}
              >
                {rowVirtualizer.getVirtualItems().map(virtualRow => {
                  const rowCredentials = credentialRows[virtualRow.index]
                  return (
                    <div
                      key={virtualRow.key}
                      ref={rowVirtualizer.measureElement}
                      data-index={virtualRow.index}
                      className="absolute left-0 top-0 grid w-full gap-4 pb-4 md:grid-cols-2 lg:grid-cols-3"
                      style={{
                        transform: `translateY(${virtualRow.start}px)`,
                      }}
                    >
                      {rowCredentials.map((credential) => (
                        <CredentialCard
                          key={credential.id}
                          credential={credential}
                          cachedBalance={cachedBalanceMap.get(credential.id)}
                          onViewBalance={handleViewBalance}
                          selected={selectedIds.has(credential.id)}
                          onToggleSelect={(event) => handleToggleSelect(credential.id, event)}
                          balance={balanceMap.get(credential.id) || null}
                          loadingBalance={loadingBalanceIds.has(credential.id)}
                        />
                      ))}
                    </div>
                  )
                })}
              </div>
            </div>
          )}
        </div>
      </main>

      {/* 余额对话框 */}
      <BalanceDialog
        credentialId={selectedCredentialId}
        open={balanceDialogOpen}
        onOpenChange={(open) => {
          setBalanceDialogOpen(open)
          if (!open) {
            setForceRefreshBalance(false)
            // 关闭弹窗时刷新缓存余额，让卡片显示最新数据
            queryClient.invalidateQueries({ queryKey: ['cached-balances'] })
          }
        }}
        forceRefresh={forceRefreshBalance}
        onBalanceLoaded={handleBalanceLoaded}
      />

      {/* 添加凭据对话框 */}
      <AddCredentialDialog
        open={addDialogOpen}
        onOpenChange={setAddDialogOpen}
      />

      {/* 导入凭据对话框 */}
      <ImportTokenJsonDialog
        open={importDialogOpen}
        onOpenChange={setImportDialogOpen}
      />

      {/* 全局配置对话框 */}
      <GlobalConfigDialog
        open={globalConfigDialogOpen}
        onOpenChange={setGlobalConfigDialogOpen}
      />


      {/* 批量验活对话框 */}
      <BatchVerifyDialog
        open={verifyDialogOpen}
        onOpenChange={setVerifyDialogOpen}
        verifying={verifying}
        progress={verifyProgress}
        results={verifyResults}
        onCancel={handleCancelVerify}
      />

      {/* 批量删除确认对话框 */}
      <Dialog open={batchDeleteDialogOpen} onOpenChange={setBatchDeleteDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认批量删除</DialogTitle>
            <DialogDescription>
              确定要删除 {pendingBatchDeleteIds.length} 个已禁用凭据吗？此操作无法撤销。
              {selectedIds.size - pendingBatchDeleteIds.length > 0 && `（将跳过 ${selectedIds.size - pendingBatchDeleteIds.length} 个未禁用凭据）`}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setBatchDeleteDialogOpen(false)}>取消</Button>
            <Button variant="destructive" onClick={confirmBatchDelete}>确认删除</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 清除已禁用确认对话框 */}
      <Dialog open={clearAllDialogOpen} onOpenChange={setClearAllDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认清除已禁用凭据</DialogTitle>
            <DialogDescription>
              确定要清除所有 {pendingClearAllCount} 个已禁用凭据吗？此操作无法撤销。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setClearAllDialogOpen(false)}>取消</Button>
            <Button variant="destructive" onClick={confirmClearAll}>确认清除</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
