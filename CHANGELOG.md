# Change Log

## 0.2.0

### Added
- **Syntax-Highlighted Terminal Output:** Added `TerminalRenderer` powered by `syntect` (`base16-ocean.dark` theme, pure-Rust `fancy-regex` backend) to display ranked search hits with line numbers and language-aware highlighting by default.
- **Machine-Readable JSON Mode (`--json`):** Added a global `--json` CLI flag to emit structured JSON payloads (`SearchPayload`) for scripting and piping while keeping human-friendly highlighted output as the default.
- **Typed Command Execution (`SearchCommand::execute`):** Exposed typed `SearchPayload` results alongside the opaque `serde_json::Value` command boundary so terminal renderers and upcoming interactive modes can consume search results without re-parsing JSON.

## 0.1.0 - Initial Release
