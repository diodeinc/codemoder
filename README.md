# Codemoder

An MCP proxy server that adds "code mode" capability to any MCP server. This lets AI models write JavaScript code that can call multiple tools in a single execution, instead of calling tools one at a time.

## How it Works

```
┌─────────────────┐     ┌──────────────────┐     ┌────────────────────┐
│   MCP Client    │────▶│    codemoder     │────▶│ Downstream MCP     │
│   (Claude, etc) │     │                  │     │ Server             │
└─────────────────┘     └──────────────────┘     └────────────────────┘
```

The proxy:
1. Spawns and connects to a downstream MCP server
2. Intercepts `list_tools` and adds an `execute_tools` tool
3. Generates TypeScript interface definitions for all tools
4. When `execute_tools` is called, runs JavaScript code that can call tools
5. Proxies regular tool calls through to the downstream server

## Usage

```bash
# Proxy any MCP server (simple form)
codemoder ./my-mcp-server

# With options, use -- to separate codemoder args from the command
codemoder --mode replace -- ./my-mcp-server

# Custom tool name
codemoder --tool-name "run_script" -- ./my-mcp-server

# Only include specific tools
codemoder --include-tools "move_items,get_footprints" -- ./my-mcp-server
```

## Options

| Option | Description | Default |
|--------|-------------|---------|
| `--mode` | `add` exposes both execute_tools and original tools; `replace` only exposes execute_tools | `add` |
| `--tool-name` | Name of the code execution tool | `execute_tools` |
| `--include-tools` | Comma-separated list of tools to include | all tools |

## Example

When connected through the proxy, the model can write:

```javascript
// Get all items and sum their values
var items = tools.get_items({}).items;
var total = 0;
for (var i = 0; i < items.length; i++) {
  total += items[i].value;
}
({count: items.length, total: total})
```

## Building

```bash
cargo build --release
```

The binary will be at `target/release/codemoder`.
