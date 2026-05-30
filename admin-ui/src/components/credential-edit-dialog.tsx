import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Loader2 } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import type { CredentialStatusItem } from '@/types/api'

interface CredentialEditDialogProps {
  field: 'priority' | 'region' | 'endpoint' | null
  credential: CredentialStatusItem
  onSave: (field: string, value: string | number | Record<string, string | null>) => void
  onClose: () => void
  isPending?: boolean
}

const FIELD_TITLES: Record<NonNullable<CredentialEditDialogProps['field']>, string> = {
  priority: '编辑优先级',
  region: '编辑 Region',
  endpoint: '编辑 Endpoint',
}

export function CredentialEditDialog({
  field,
  credential,
  onSave,
  onClose,
  isPending = false,
}: CredentialEditDialogProps) {
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [regionValue, setRegionValue] = useState(credential.region ?? '')
  const [apiRegionValue, setApiRegionValue] = useState(credential.apiRegion ?? '')
  const [endpointValue, setEndpointValue] = useState(credential.endpoint ?? '')

  useEffect(() => {
    if (field) {
      setPriorityValue(String(credential.priority))
      setRegionValue(credential.region ?? '')
      setApiRegionValue(credential.apiRegion ?? '')
      setEndpointValue(credential.endpoint ?? '')
    }
  }, [field, credential])

  const handleSave = () => {
    if (!field) return
    switch (field) {
      case 'priority': {
        const parsed = parseFloat(priorityValue)
        if (isNaN(parsed) || parsed < 0) {
          toast.error('优先级必须是非负数')
          return
        }
        onSave('priority', parsed)
        break
      }
      case 'region': {
        onSave('region', {
          region: regionValue.trim() || null,
          apiRegion: apiRegionValue.trim() || null,
        })
        break
      }
      case 'endpoint': {
        const trimmed = endpointValue.trim()
        if (!trimmed) {
          toast.error('Endpoint 不能为空')
          return
        }
        onSave('endpoint', trimmed)
        break
      }
    }
  }

  const handleOpenChange = (open: boolean) => {
    if (!open) onClose()
  }

  const isOpen = field !== null

  return (
    <Dialog open={isOpen} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{field ? FIELD_TITLES[field] : ''}</DialogTitle>
        </DialogHeader>

        <div className="space-y-3">
          {field === 'priority' && (
            <div>
              <label className="text-xs text-muted-foreground mb-1 block">优先级</label>
              <Input
                type="number"
                min="0"
                step="1"
                value={priorityValue}
                onChange={(e) => setPriorityValue(e.target.value)}
                placeholder="输入非负整数"
                disabled={isPending}
                autoFocus
              />
            </div>
          )}

          {field === 'region' && (
            <div className="space-y-2">
              <div>
                <label className="text-xs text-muted-foreground mb-1 block">Auth Region</label>
                <Input
                  value={regionValue}
                  onChange={(e) => setRegionValue(e.target.value)}
                  placeholder="留空使用全局默认"
                  disabled={isPending}
                  autoFocus
                />
              </div>
              <div>
                <label className="text-xs text-muted-foreground mb-1 block">API Region</label>
                <Input
                  value={apiRegionValue}
                  onChange={(e) => setApiRegionValue(e.target.value)}
                  placeholder="留空使用 Auth Region"
                  disabled={isPending}
                />
              </div>
            </div>
          )}

          {field === 'endpoint' && (
            <div>
              <label className="text-xs text-muted-foreground mb-1 block">Endpoint</label>
              <Input
                value={endpointValue}
                onChange={(e) => setEndpointValue(e.target.value)}
                placeholder={credential.effectiveEndpoint}
                disabled={isPending}
                autoFocus
              />
            </div>
          )}
        </div>

        <DialogFooter>
          <Button
            variant="outline"
            onClick={onClose}
            disabled={isPending}
          >
            取消
          </Button>
          <Button
            onClick={handleSave}
            disabled={isPending}
          >
            {isPending && <Loader2 className="h-3 w-3 mr-1 animate-spin" />}
            保存
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
