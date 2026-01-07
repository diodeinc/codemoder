# Codemoder Development Guide

## Build Commands

```bash
cargo build                    # Debug build
cargo build --release          # Release build
cargo test                     # Run all tests
cargo test -- --nocapture      # Run tests with output
cargo clippy                   # Lint
cargo fmt                      # Format code
```

## Testing

Integration tests require the mock server to be built first:

```bash
cargo build --bin mock-mcp-server
cargo test
```

## Architecture

- `src/main.rs` - CLI entry point with clap argument parsing
- `src/lib.rs` - Public exports
- `src/config.rs` - Configuration types (`CodeModeConfig`, `CodeModeExposure`)
- `src/proxy.rs` - MCP proxy implementation (`CodeModeProxy`)
- `src/runtime.rs` - QuickJS JavaScript runtime for executing code
- `src/typescript.rs` - TypeScript interface generation from JSON Schema
- `src/bin/mock_server.rs` - Mock MCP server for testing

## Key Concepts

### Proxy Flow
1. Client connects to codemoder via stdio
2. Codemoder spawns downstream MCP server as child process
3. `list_tools` adds `execute_tools` to the tool list
4. `execute_tools` runs JavaScript in QuickJS with `tools.*` bindings
5. Other tool calls are proxied directly to downstream

### JavaScript Runtime
- Uses `rquickjs` (QuickJS bindings for Rust)
- Tools are exposed synchronously via `tools.name({args})`
- `console.log()` captures output and returns it with results
- Errors in JS or tool calls propagate back to the client

## Code Style

- Use `anyhow::Result` for error handling
- Follow existing patterns in the codebase
- Add tests for new functionality
