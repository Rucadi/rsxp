# Fanplay MCP and OpenAPI

`fanplay-mcp` provides a stdio MCP interface to a running Fanplay local control
API. It discovers operations and extension routes over HTTP and maps them to
MCP tools. The game runs in the existing player process, not in the MCP binary.

```sh
cargo build --release -p fanplay-mcp
cargo api-openapi > crates/fanplay-mcp/openapi.json
```

Example client configuration:

```json
{
  "mcpServers": {
    "fanplay": {
      "command": "/absolute/path/to/target/release/fanplay-mcp",
      "env": { "FANPLAY_API_HOST": "127.0.0.1", "FANPLAY_API_PORT": "47821" }
    }
  }
}
```

Start the desktop player with its API enabled before calling game tools. The
launcher enables the local API by default; `FANPLAY_API=0` disables it. Game
operations also need an active compatible session and the relevant plugins.
The browser's `window.FANPLAY.control` adapter is not a TCP endpoint for this
MCP executable.

`openapi.json` is generated from `fanplay-control::Catalog::standard()`.
Use `/v1/discovery` or `/v1/openapi.json` on the live player to inspect its
available operations and loaded extension routes. The static file is not a
promise that every operation is executable without a game.

See [control API](../../docs/control-api.md) for transport/ownership,
[plugins](../../extensions/README.md) for game-specific operations, and
[`main.rs`](src/main.rs) for the MCP implementation.
