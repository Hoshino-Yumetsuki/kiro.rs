import { useState } from 'react'
import { toast } from 'sonner'
import { Loader2 } from 'lucide-react'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import type { CredentialStatusItem } from '@/types/api'

interface CredentialEditPopoverProps {
  field: 'priority' | 'region' | 'endpoint'
  credential: CredentialStatusItem
  onSave: (field: string, value: string | number | Record<string, string | null>) => void
  isPending?: boolean
  trigger: React.ReactNode
  defaultOpen?: boolean
  onClose?: () => void
}

export function CredentialEditPopover({
  field,
  credential,
  onSave,
  isPending = false,
  trigger,
  defaultOpen = false,
  onClose,
}: CredentialEditPopoverProps) {
  const [open, setOpen] = useState(defaultOpen)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [regionValue, setRegionValue] = useState(credential.region ?? '')
  const [apiRegionValue, setApiRegionValue] = useState(credential.apiRegion ?? '')
  const [endpointValue, setEndpointValue] = useState(credential.endpoint ?? '')

  const handleOpenChange = (newOpen: boolean) => {
    if (newOpen) {
      setPriorityValue(String(credential.priority))
      setRegionValue(credential.region ?? '')
      setApiRegionValue(credential.apiRegion ?? '')
      setEndpointValue(credential.endpoint ?? '')
    }
    setOpen(newOpen)
    if (!newOpen) onClose?.()
  }

  const handleSave = () => {
    switch (field) {
      case 'priority': {
        const parsed = parseFloat(priorityValue)
        if (isNaN(parsed) || parsed < 0) {
          toast.error('优先级必须是非负数')
          return
        }
        onSave('priority', parsed)
        setOpen(false)
        break
      }
      case 'region': {
        onSave('region', {
          region: regionValue.trim() || null,
          apiRegion: apiRegionValue.trim() || null,
        })
        setOpen(false)
        break
      }
      case 'endpoint': {
        const trimmed = endpointValue.trim()
        if (!trimmed) {
          toast.error('Endpoint 不能为空')
          return
        }
        onSave('endpoint', trimmed)
        setOpen(false)
        break
      }
    }
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>{trigger}</PopoverTrigger>
      <PopoverContent className="w-72" align="start">
        <div className="space-y-3">
          <h4 className="font-medium text-sm">
            {field === 'priority' && '编辑优先级'}
            {field === 'region' && '编辑 Region'}
            {field === 'endpoint' && '编辑 Endpoint'}
          </h4>

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
              />
            </div>
          )}

          <div className="flex justify-end gap-2 pt-1">
            <Button
              size="sm"
              variant="outline"
              onClick={() => setOpen(false)}
              disabled={isPending}
            >
              取消
            </Button>
            <Button
              size="sm"
              onClick={handleSave}
              disabled={isPending}
            >
              {isPending && <Loader2 className="h-3 w-3 mr-1 animate-spin" />}
              保存
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  )
}
