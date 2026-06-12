# AGENTS.md

## Project Overview

kiro-rs is a Rust proxy service that converts Anthropic Claude API requests into Kiro API requests. It provides Anthropic Messages API compatibility with multi-credential management, automatic failover, streaming responses, and an optional web admin UI.

**Tech stack**: Rust 2024 edition (Axum 0.8 + Tokio) for the backend; React 19 + TypeScript + Tailwind CSS 4 + Vite 8 for the admin UI frontend.

**Current version**: Defined in both `Cargo.toml` (`version`) and `/VERSION` (kept in sync).

## Setup

### Prerequisites

- Rust stable toolchain (edition 2024)
- Node.js 22+
- pnpm 11+

### Build

The admin UI must be built **before** the Rust backend because static files are embedded into the binary via `rust-embed`.

```bash
# Build frontend first, then backend (Makefile handles ordering)
make release

# Or manually:
cd admin-ui && pnpm install && pnpm build
cd ..
cargo build --release
```

### Run

```bash
# Production (embedded admin UI)
./target/release/kiro-rs -c config.json --credentials credentials.json

# Development mode (Vite dev server + Rust backend, hot reload for frontend)
make dev
# Frontend: http://localhost:5173/admin/  (proxies /api to :8990)
# Backend:  http://localhost:8990
```

### Docker

```bash
docker-compose up
# Mount config.json and credentials.json into ./config/
```

## Development Commands

```bash
make help       # List all targets
make dev        # Frontend dev server + Rust backend (sensitive-logs enabled)
make run        # Build frontend, run backend
make build      # Build frontend + backend (debug)
make release    # Build frontend + backend (release)
make ui         # Build frontend only
make ui-dev     # Frontend dev server only

make test       # cargo test
make lint       # cargo clippy -- -D warnings
make fmt        # cargo fmt
make check      # fmt + clippy + test (run before committing)

make docker     # Build Docker image
make clean      # Remove target/, admin-ui/dist/, admin-ui/node_modules/
```

### Running with sensitive logs

The `sensitive-logs` feature flag enables diagnostic output (token usage, request body sizes). Off by default. Use only for troubleshooting.

```bash
cargo run --features sensitive-logs -- -c config.json --credentials credentials.json
RUST_LOG=debug ./target/release/kiro-rs   # control log level via env
```

## Testing

```bash
cargo test                    # Run all tests
cargo test <test_name>        # Run a specific test
```

- Integration tests live in `tests/` (e.g. `common_smoke.rs`, `tool_choice_warning.rs`).
- Shared test helpers are in `tests/common/`.
- There are no Rust unit test modules inside `src/` — tests are external only.
- `tools/test_prompt_cache_usage.mjs` is a Node.js script for manual prompt-cache scenario testing.

## Code Style

- **Formatter**: `cargo fmt` (default rustfmt settings — no `rustfmt.toml` override).
- **Linter**: `cargo clippy -- -D warnings` (treat all warnings as errors).
- **No `rustfmt.toml` or `clippy.toml`** — uses Rust defaults.
- **Frontend**: TypeScript strict mode (`noUnusedLocals`, `noUnusedParameters`). Path alias `@/*` → `./src/*`.
- Run `make check` (fmt + clippy + test) before committing.

## Architecture

### Source Layout

```
src/
├── main.rs                   # Entry point, CLI arg parsing, server startup
├── lib.rs                    # Crate root — re-exports modules for integration tests
├── http_client.rs            # HTTP client builder (reqwest, proxy support)
├── image.rs                  # Image processing (resize, GIF frame extraction)
├── pdf.rs                    # PDF text extraction
├── token.rs                  # Token counting / estimation
├── model/
│   ├── config.rs             # AppConfig struct (config.json deserialization)
│   └── arg.rs                # CLI argument definitions (clap)
├── anthropic/                # Anthropic API compatibility layer
│   ├── router.rs             # Axum route definitions (/v1/*)
│   ├── handlers.rs           # Request handlers (messages, count_tokens, models)
│   ├── middleware.rs          # Auth middleware (x-api-key / Bearer)
│   ├── types.rs              # Anthropic request/response types
│   ├── converter.rs          # Anthropic ↔ Kiro protocol conversion
│   ├── stream.rs             # AWS Event Stream → Anthropic SSE transform
│   ├── websearch.rs          # WebSearch tool handling
│   ├── compressor.rs         # Multi-layer input compression pipeline
│   ├── cache_tracker.rs      # Local prompt cache simulation
│   ├── rewriter.rs           # Request rewriting
│   ├── structured_output.rs  # Structured output fence stripping
│   ├── tool_compression.rs   # Tool definition compression
│   ├── truncation.rs         # Message history truncation
│   └── usage.rs              # Usage/billing tracking
├── kiro/                     # Kiro API client
│   ├── provider.rs           # API provider (request dispatch, retry, failover)
│   ├── token_manager.rs      # Multi-credential token lifecycle
│   ├── cooldown.rs           # Credential cooldown/circuit-breaker
│   ├── rate_limiter.rs       # Per-credential rate limiting
│   ├── affinity.rs           # Credential affinity routing
│   ├── background_refresh.rs # Async token refresh
│   ├── fingerprint.rs        # Device fingerprint generation
│   ├── machine_id.rs         # Machine ID generation
│   ├── web_portal.rs         # Web portal / balance queries
│   ├── endpoint/             # Endpoint definitions
│   ├── model/                # Kiro request/response/event types
│   └── parser/               # AWS Event Stream binary parser (frames, headers, CRC32C)
├── admin/                    # Admin API
│   ├── router.rs             # Admin route definitions (/api/admin/*)
│   ├── handlers.rs           # Admin request handlers
│   ├── service.rs            # Admin business logic
│   ├── middleware.rs          # Admin auth middleware
│   ├── types.rs              # Admin request/response types
│   └── error.rs              # Admin error types
├── admin_ui/                 # Embedded static file serving (rust-embed)
│   └── router.rs             # Static file route handler
└── common/                   # Shared utilities
    ├── auth.rs               # Auth helper functions
    ├── redact.rs             # Log redaction
    └── utf8.rs               # UTF-8 utilities
```

### Admin UI Frontend (`admin-ui/`)

React 19 SPA for credential management. Uses:
- Vite 8 + `@vitejs/plugin-react-swc`
- Tailwind CSS 4 + Radix UI primitives
- TanStack React Query for data fetching
- shadcn/ui component patterns (`components.json` present)

Built output (`admin-ui/dist/`) is embedded into the Rust binary at compile time via `rust-embed`.

### Request Processing Flow

```
POST /v1/messages (Anthropic format)
  → auth_middleware: validate x-api-key or Bearer token (constant-time comparison via subtle)
  → post_messages handler:
      1. Check WebSearch trigger conditions
      2. converter::convert_request() → Kiro request format
      3. provider.call_api() → send to Kiro (with retry + failover)
      4. stream.rs: parse AWS Event Stream → convert to Anthropic SSE
```

### Core Design Patterns

1. **Provider Pattern** (`kiro/provider.rs`): Unified API dispatch with per-credential HTTP client caching and proxy support.
2. **Multi-Token Manager** (`kiro/token_manager.rs`): Priority-based credential management with async background refresh.
3. **Protocol Converter** (`anthropic/converter.rs`): Bidirectional Anthropic ↔ Kiro conversion including model mapping, JSON Schema normalization, image format conversion.
4. **Event Stream Parser** (`kiro/parser/`): AWS Event Stream binary protocol (header + payload + CRC32C).
5. **Input Compressor** (`anthropic/compressor.rs`): Multi-layer compression pipeline (whitespace → thinking truncation → tool_result truncation → tool_use input truncation → history truncation) with automatic tool_use/tool_result pairing repair.
6. **Credential Cooldown** (`kiro/cooldown.rs`): Category-based cooldown (FailureLimit / InsufficientBalance / ModelUnavailable / QuotaExceeded) with global circuit-breaker.

### Shared State

```rust
AppState {
    api_key: String,                          // Client-facing API key
    kiro_provider: Option<Arc<KiroProvider>>,  // Core API provider (Arc shared)
    profile_arn: Option<String>,               // AWS Profile ARN
    compression_config: CompressionConfig,     // Input compression settings
}
```

Injected via Axum `State` extractor into all handlers.

## CI/CD

GitHub Actions workflows in `.github/workflows/`:

- **`build.yaml`**: Main build pipeline. Triggers on `master` push and version tags (`v*`). Builds for 7 targets: macOS arm64/x64, Windows x64, Linux x64/arm64, Linux musl x64/arm64. Builds admin-ui first (Node 22, pnpm 11), then cargo build.
- **`release-linux.yml`** / **`release-macos.yml`** / **`release-windows.yml`**: Platform-specific release workflows.
- **`docker-build.yml`** / **`docker-test-image.yml`**: Docker image build and test.

## Cargo Features

| Feature | Description |
|---|---|
| `sensitive-logs` | Enables diagnostic logging of token usage and request sizes. Off by default. |
| `native-tls` | Uses `native-tls` instead of `rustls` for TLS. Switchable at runtime via `config.json` `tlsBackend`. |

## Important Notes

1. **Build order matters**: Always build `admin-ui` before compiling the Rust binary. The `Makefile` handles this automatically.
2. **No rustfmt.toml or clippy.toml**: Project uses Rust default formatting and linting rules.
3. **Rust 2024 edition**: Uses the latest Rust edition. Check `Cargo.toml` `edition = "2024"`.
4. **Credential files are gitignored**: `config.json`, `credentials.json`, and `config/` are in `.gitignore`. Example configs are provided as `config.example.json`, `credentials.example.*.json`.
5. **Retry strategy**: Single credential retries up to 2 times; single request retries up to 3 times across credentials.
6. **Security**: `subtle` crate for constant-time API key comparison. Admin API disabled when `adminApiKey` is empty/missing.
7. **Image processing**: GIFs are frame-extracted to JPEG sequences (max 20 frames, max 5fps). Images over 4000px long edge or 4M total pixels are downscaled.
8. **Input compression**: Automatically triggered when request body approaches ~5MB upstream limit.
9. **TLS backend**: Defaults to `rustls`. Switch to `native-tls` via `config.json` `tlsBackend` field if encountering certificate or proxy issues.
