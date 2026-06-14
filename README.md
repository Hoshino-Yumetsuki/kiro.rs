# kiro.rs

Rust 编写的 Anthropic Claude API 代理服务。将 Anthropic Messages API 请求转换为 Kiro API 请求，支持多凭据管理、自动故障转移、流式响应和可选的 Web 管理界面。

---

> [!WARNING]
> 本项目仅供研究使用。Use at your own risk，使用本项目所导致的任何后果由使用者承担，与本项目无关。
> 本项目与 AWS / Kiro / Anthropic / Claude 官方无关，不代表官方立场。

## 功能

- Anthropic Messages API 兼容（`/v1/messages`、`/v1/messages/count_tokens`、`/v1/models`）
- SSE 流式响应
- OAuth Token 自动刷新与多凭据故障转移
- Thinking 模式（extended thinking）与工具调用（tool use）
- 内置 WebSearch 工具转换
- 多模型支持：Claude + 非 Claude 系列，模型列表可通过 config 配置
- 凭据等级路由：自动将 Premium 模型请求路由到 Pro 以上凭据，无匹配时提前拒绝（403）
- 本地 Claude tokenizer 计数（用于 `/v1/messages/count_tokens` 与 usage 统计）
- 多级 Region 配置与凭据级代理
- 凭据冷却、速率限制、粘性路由
- 请求体自动压缩（白空格压缩、thinking 截断、工具定义压缩、自适应压缩）
- 本地 Prompt Cache 模拟记账
- 可选 Web 管理界面（凭据管理、余额查询、配置热更新）

## 快速开始

### 安装

从 [Releases](https://github.com/Hoshino-Yumetsuki/kiro.rs/releases) 下载预编译二进制文件，或自行编译：

```bash
# 前端 Admin UI 需要先构建（嵌入到二进制中）
cd admin-ui && pnpm install && pnpm build && cd ..

# 编译后端
cargo build --release
```

> [!NOTE]
> 编译需要 Rust stable（edition 2024）、Node.js 22+、pnpm 11+。
> 也可以用 `make release` 一步完成前端 + 后端编译。

### 配置

创建 `config.json`：

```json
{
  "host": "127.0.0.1",
  "port": 8990,
  "apiKey": "sk-kiro-rs-your-api-key",
  "region": "us-east-1"
}
```

创建 `credentials.json`（从 Kiro IDE 获取凭证信息）：

**Social 认证：**

```json
{
  "refreshToken": "你的刷新token",
  "expiresAt": "2025-12-31T02:32:45.144Z",
  "authMethod": "social"
}
```

**IdC 认证：**

```json
{
  "refreshToken": "你的刷新token",
  "expiresAt": "2025-12-31T02:32:45.144Z",
  "authMethod": "idc",
  "clientId": "你的clientId",
  "clientSecret": "你的clientSecret"
}
```

**API Key 认证：**

```json
{
  "kiroApiKey": "ksk_your_api_key_here",
  "authMethod": "api_key"
}
```

> [!TIP]
> 也可以先启动服务，通过 Web 管理面板在线添加凭据（需配置 `adminApiKey`）。

### 启动

```bash
./target/release/kiro-rs
```

指定配置文件路径：

```bash
./target/release/kiro-rs -c /path/to/config.json --credentials /path/to/credentials.json
```

### 验证

```bash
curl http://127.0.0.1:8990/v1/messages \
  -H "Content-Type: application/json" \
  -H "x-api-key: sk-kiro-rs-your-api-key" \
  -d '{
    "model": "claude-sonnet-4-5",
    "max_tokens": 1024,
    "stream": true,
    "messages": [{"role": "user", "content": "Hello, Claude!"}]
  }'
```

### Docker

```bash
docker-compose up
```

将 `config.json` 和 `credentials.json` 放入 `./config/` 目录，容器会自动挂载。

## 配置

### config.json

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `host` | string | `127.0.0.1` | 监听地址 |
| `port` | number | `8080` | 监听端口 |
| `apiKey` | string | — | 客户端认证用的 API Key（必填） |
| `region` | string | `us-east-1` | AWS 区域 |
| `authRegion` | string | — | Token 刷新用的区域，未配置回退到 `region` |
| `apiRegion` | string | — | API 请求用的区域，未配置回退到 `region` |
| `kiroVersion` | string | `0.11.107` | Kiro 版本号 |
| `machineId` | string | 自动生成 | 机器码（64 位十六进制） |
| `tlsBackend` | string | `rustls` | TLS 后端：`rustls` 或 `native-tls` |
| `proxyUrl` | string | — | 全局代理地址（http/https/socks5） |
| `proxyUsername` | string | — | 代理用户名 |
| `proxyPassword` | string | — | 代理密码 |
| `adminApiKey` | string | — | Admin API 密钥，配置后启用管理界面 |
| `defaultEndpoint` | string | `ide` | 默认 Kiro 端点 |
| `credentialRpm` | number | — | 单凭据目标 RPM（每分钟请求数） |
| `promptCacheTtlSeconds` | number | `300` | 本地 Prompt Cache TTL（秒） |
| `promptCacheMode` | string | `simulated` | Prompt Cache usage 模式：`upstream` / `simulated` / `off`（兼容旧字段 `promptCacheAccountingEnabled`） |
| `extractThinking` | bool | `true` | 非流式响应中提取 `<thinking>` 为独立内容块 |
| `enableCredentialCooldown` | bool | `true` | 凭据冷却机制 |
| `enableRateLimit` | bool | `true` | 速率限制节流 |
| `enableStickyRouting` | bool | `true` | 粘性路由（同 session 请求路由到同一凭据） |
| `autoDisableInsufficientBalance` | bool | `true` | 余额不足时自动禁用凭据 |
| `autoDisableRefreshFailure` | bool | `true` | Token 刷新失败时自动禁用凭据 |
| `autoDisableOnForbidden` | bool | `true` | 上游返回 403 时自动禁用凭据 |
| `countTokensApiUrl` | string | — | 外部 count_tokens API 地址 |
| `countTokensApiKey` | string | — | 外部 count_tokens API 密钥 |
| `countTokensAuthType` | string | `x-api-key` | 外部 API 认证类型：`x-api-key` 或 `bearer` |
| `supportedTiers` | string[] | `[]` | 部署支持的凭据等级列表（如 `["free", "pro", "pro+"]`），空数组不过滤 |
| `models` | [ModelConfig](#models) | 见默认值 | 模型列表配置，可自定义 Anthropic ID 到 Kiro ID 的映射及等级限制 |

#### compression（输入压缩配置）

```json
{
  "compression": {
    "enabled": true,
    "whitespaceCompression": true,
    "thinkingStrategy": "discard",
    "toolDescriptionMaxChars": 4000,
    "toolDefinitionCompression": true,
    "toolDefinitionMinDescriptionChars": 50,
    "toolNameMaxChars": 63,
    "maxRequestBodyBytes": 4718592,
    "adaptiveCompression": true,
    "adaptiveCompressionMaxIters": 32
  }
}
```

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `enabled` | `true` | 压缩总开关 |
| `whitespaceCompression` | `true` | 压缩连续空行、行尾空格 |
| `thinkingStrategy` | `discard` | thinking 块处理：`discard` / `truncate` / `keep` |
| `toolDescriptionMaxChars` | `4000` | 工具描述截断阈值（字符数） |
| `toolDefinitionCompression` | `true` | 工具定义 schema 简化 + 描述截断 |
| `toolDefinitionMinDescriptionChars` | `50` | 压缩后描述最少保留字符数 |
| `toolNameMaxChars` | `63` | 工具名最大长度（0 = 不限） |
| `maxRequestBodyBytes` | `4718592` | 请求体上限（约 4.5MB），超过触发自适应压缩 |
| `adaptiveCompression` | `true` | 自适应压缩开关 |
| `adaptiveCompressionMaxIters` | `32` | 自适应压缩最大迭代次数 |

#### rewriter（响应关键词改写）

```json
{
  "rewriter": {
    "enabled": false,
    "keywords": ["Kiro"],
    "rewritePrompt": "...",
    "maxRewriteTokens": 16384
  }
}
```

默认关闭。启用后，当响应文本包含指定关键词时，会触发二次改写。

### credentials.json

支持单对象（向后兼容）或 JSON 数组（多凭据）格式。

| 字段 | 类型 | 说明 |
|------|------|------|
| `refreshToken` | string | OAuth 刷新令牌 |
| `accessToken` | string | OAuth 访问令牌（可选，可自动刷新） |
| `expiresAt` | string | Token 过期时间（RFC3339） |
| `authMethod` | string | 认证方式：`social` / `idc` / `api_key` |
| `kiroApiKey` | string | Kiro API Key（`api_key` 认证方式必填） |
| `clientId` | string | IdC 客户端 ID（`idc` 必填） |
| `clientSecret` | string | IdC 客户端密钥（`idc` 必填） |
| `priority` | number | 优先级，数字越小越优先（默认 0） |
| `region` | string | 凭据级 Auth Region（兼容字段） |
| `authRegion` | string | 凭据级 Auth Region |
| `apiRegion` | string | 凭据级 API Region |
| `machineId` | string | 凭据级机器码 |
| `endpoint` | string | 凭据级端点名称 |
| `proxyUrl` | string | 凭据级代理（`direct` 表示不使用代理） |
| `proxyUsername` | string | 凭据级代理用户名 |
| `proxyPassword` | string | 凭据级代理密码 |
| `disabled` | bool | 是否禁用该凭据 |
| `tier` | string | 凭据等级：`free` / `pro` / `pro+`。未设置则匹配所有等级（向后兼容） |

> **IdC / Builder-ID / IAM** 统一使用 `authMethod: "idc"`。旧值 `builder-id` / `iam` 仍可识别。

#### 多凭据示例

```json
[
  {
    "refreshToken": "第一个凭据",
    "authMethod": "social",
    "priority": 0,
    "tier": "pro+"
  },
  {
    "refreshToken": "第二个凭据",
    "authMethod": "idc",
    "clientId": "xxx",
    "clientSecret": "xxx",
    "region": "us-east-2",
    "priority": 1,
    "tier": "free",
    "proxyUrl": "socks5://proxy.example.com:1080"
  }
]
```

- 按 `priority` 排序，数字越小优先级越高
- 单凭据最多重试 2 次，单请求跨凭据最多重试 3 次
- 多凭据格式下 Token 刷新后自动回写到文件
- 配置 `tier` 后，Premium 模型（Opus / Sonnet 4.6 / Haiku）仅路由到匹配等级的凭据，Free 凭据不会被选中

### Region 配置

分别控制 Token 刷新和 API 请求使用的区域。

**Auth Region** 优先级：`凭据.authRegion` > `凭据.region` > `config.authRegion` > `config.region`

**API Region** 优先级：`凭据.apiRegion` > `config.apiRegion` > `config.region`

### 代理配置

优先级：`凭据.proxyUrl` > `config.proxyUrl` > 无代理

| 凭据 proxyUrl | 行为 |
|---|---|
| 具体 URL（`http://...`、`socks5://...`） | 使用凭据指定的代理 |
| `direct` | 显式不走代理 |
| 未配置 | 回退到全局代理 |

### 客户端认证

请求本服务时支持两种方式：

```
x-api-key: sk-your-api-key
```

```
Authorization: Bearer sk-your-api-key
```

## API

### 标准端点

| 端点 | 方法 | 说明 |
|------|------|------|
| `/v1/models` | GET | 可用模型列表 |
| `/v1/messages` | POST | 创建消息 |
| `/v1/messages/count_tokens` | POST | 估算 token 数 |

### Token 计数与 usage

本服务优先使用内置 Claude BPE tokenizer 在本地计算输入 token，用于：

- `/v1/messages/count_tokens`
- `/v1/messages` 响应中的 `usage.input_tokens`
- 本地 Prompt Cache 模拟记账

Kiro 上游的 `contextUsageEvent` 只提供上下文窗口百分比，且会包含 Kiro 注入的内部上下文，因此不会直接作为下游 `input_tokens`。只有本地计数不可用时，才会把该百分比按模型上下文窗口转换为 fallback，并扣除观测到的 Kiro 内部上下文开销。

`promptCacheMode` 控制 Prompt Cache usage 字段：

| 模式 | 行为 |
|---|---|
| `upstream` | 不输出 cache 字段；`input_tokens` 使用本地 Claude tokenizer 计数，必要时回退到上游百分比推断 |
| `simulated` | 在相同 `input_tokens` 基础上，用本地 `CacheTracker` 模拟 `cache_creation_input_tokens` / `cache_read_input_tokens` |
| `off` | 不输出 cache 字段，也不进行本地 cache 记账 |

### Thinking 模式

通过请求体控制，与模型名无关：

```json
{
  "model": "claude-opus-4-7",
  "thinking": {"type": "adaptive"},
  "output_config": {"effort": "high"},
  "max_tokens": 16000,
  "messages": [...]
}
```

- `thinking.type`：`adaptive`（启用）/ `enabled`（旧版兼容）/ `disabled`（关闭）
- `output_config.effort`：`low` / `medium` / `high`（默认）/ `xhigh` / `max`
  - `xhigh` 和 `max` 仅 Opus 系列支持

### 工具调用

完整支持 Anthropic tool use：

```json
{
  "model": "claude-sonnet-4-20250514",
  "max_tokens": 1024,
  "tools": [
    {
      "name": "get_weather",
      "description": "获取城市天气",
      "input_schema": {
        "type": "object",
        "properties": {"city": {"type": "string"}},
        "required": ["city"]
      }
    }
  ],
  "messages": [...]
}
```

### 模型配置

模型列表通过 `config.json` 的 `models` 字段配置，支持自定义 Anthropic ID → Kiro ID 映射及等级限制。不配置则使用内置默认列表（含所有 Claude + 非 Claude 模型）。

每个模型条目包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | Anthropic 风格的模型 ID（如 `"claude-sonnet-4-6"`） |
| `displayName` | string | 显示名称（如 `"Claude Sonnet 4.6"`） |
| `createdAt` | number | 创建时间（Unix 时间戳） |
| `kiroModelId` | string | Kiro 上游使用的模型 ID（如 `"claude-sonnet-4.6"`） |
| `tiers` | string[] | 可访问此模型的凭据等级列表（如 `["pro", "pro+"]`），空数组不限制 |

`GET /v1/models` 仅返回 `tiers` 与 `supportedTiers` 有交集的模型。

**等级路由**：请求到达时，按模型的 `tiers` 过滤凭据。若凭据设置了 `tier` 字段但不在模型的 `tiers` 列表中，该凭据会被跳过。未设置 `tier` 的凭据可匹配任意模型（向后兼容）。若无可用凭据匹配所需等级，返回 403。

## Admin 管理界面

配置 `adminApiKey` 后启用。访问 `/admin` 打开 Web 管理界面。

### Admin API 端点

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/admin/credentials` | GET | 获取所有凭据状态 |
| `/api/admin/credentials` | POST | 添加新凭据 |
| `/api/admin/credentials/import-token-json` | POST | 批量导入 token.json |
| `/api/admin/credentials/balances/cached` | GET | 获取所有凭据缓存余额 |
| `/api/admin/credentials/:id` | DELETE | 删除凭据 |
| `/api/admin/credentials/:id/disabled` | POST | 设置禁用状态 |
| `/api/admin/credentials/:id/priority` | POST | 设置优先级 |
| `/api/admin/credentials/:id/region` | POST | 设置 Region |
| `/api/admin/credentials/:id/endpoint` | POST | 设置端点 |
| `/api/admin/credentials/:id/reset` | POST | 重置失败计数 |
| `/api/admin/credentials/:id/refresh` | POST | 强制刷新 Token |
| `/api/admin/credentials/:id/balance` | GET | 查询凭据余额 |
| `/api/admin/proxy` | GET/POST | 代理配置查看/更新 |
| `/api/admin/config/global` | GET/PUT | 全局配置查看/更新 |

## 兼容性说明

与 Anthropic 官方 API 的已知差异：

- **不支持的 API**：Batches、Files、Managed Agents 等
- **System prompt**：转换为 user/assistant 消息对传递给上游
- **Assistant prefill**：不支持
- **tool_choice**：被忽略，响应头返回 `x-anthropic-compat-warning` 提示
- **Prompt cache**：本地模拟实现，非原生 prompt caching
- **图片 URL**：不支持 `image.source.type: "url"` 形式
- **Token 计数**：文本计数使用内置 Claude tokenizer；图片、PDF、上游内部上下文与真实 Claude 服务端计数仍可能存在差异

每个请求生成唯一 request ID，出现在响应体 `request_id` 字段和 `x-request-id` 响应头中。

## TLS 故障排查

> [!IMPORTANT]
> TLS 默认使用 `rustls`。如果遇到请求报错（特别是 token 刷新失败），尝试在 `config.json` 中将 `tlsBackend` 切换为 `native-tls`。
> 使用 HTTP 代理时，可能需要额外安装证书才能配合 `rustls` 工作。

## 技术栈

- **后端**：Rust 2024 edition — Axum 0.8 + Tokio + Reqwest + Serde + Clap + tracing
- **前端**：React 19 + TypeScript + Tailwind CSS 4 + Vite 8 + TanStack Query + Radix UI

## 致谢

本项目参考了以下项目的实现：

- [kiro2api](https://github.com/caidaoli/kiro2api)
- [proxycast](https://github.com/aiclientproxy/proxycast)
- [kiro.rs](https://github.com/hank9999/kiro.rs)
