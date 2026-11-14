# Change Log

## 0.3.0

### Added
- **Interactive REPL Mode (`ReplCommand`):** Added a `rustyline`-powered interactive loop (`blite> `) that keeps both the Tantivy RAM index and `syntect` syntax definitions hot in memory for sub-15ms queries.
- **Zero-Subcommand Default:** Running `blite` without a subcommand now launches directly into interactive mode (supporting top-level `--repo` and `--limit` flags), alongside the explicit `blite repl` subcommand.
- **Session Control Commands:** Added `:reload` (hot-rebuilds the repository index in place), `:limit <N>`, `:help`, and `:exit`/`:quit` (with clean `Ctrl+C` and `Ctrl+D` handling).

## 0.2.0

### Added
- **Syntax-Highlighted Terminal Output:** Added `TerminalRenderer` powered by `syntect` (`base16-ocean.dark` theme, pure-Rust `fancy-regex` backend) to display ranked search hits with line numbers and language-aware highlighting by default.
- **Machine-Readable JSON Mode (`--json`):** Added a global `--json` CLI flag to emit structured JSON payloads (`SearchPayload`) for scripting and piping while keeping human-friendly highlighted output as the default.
- **Typed Command Execution (`SearchCommand::execute`):** Exposed typed `SearchPayload` results alongside the opaque `serde_json::Value` command boundary so terminal renderers and upcoming interactive modes can consume search results without re-parsing JSON.

## 0.1.0 - Initial Release
