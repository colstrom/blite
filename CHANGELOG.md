# Change Log

## 0.8.3

### Added
- **Kubernetes Annotation Indexing (`annotations:`):** Extracted `metadata.annotations` from Kubernetes manifests into `K8sMetadata` and indexed them in Tantivy so queries can filter by `annotations:` alongside `labels:`.
- **Key-Only & Key-Value Metadata Filtering:** Indexed both bare keys and `key=value` pairs for `labels:` and `annotations:`, allowing queries to match any value for a given key (e.g. `annotations:cert-manager.io/cluster-issuer`) as well as exact `key=value` pairs.
- **REPL Tab-Completion (`ReplHelper`):** Added interactive tab-completion for `:commands`, field prefixes (`kind:`, `ns:`, `labels:`, `annotations:`), indexed Kubernetes kinds/namespaces, and label/annotation keys and `key=value` pairs (with automatic quoting for special characters and live updates on `:reload`).

## 0.8.2

### Added
- **Persistent REPL History (`dirs` + `camino`):** Added cross-session command history persistence for interactive mode (`blite` / `blite repl`), defaulting to the platform state/data directory (`blite/history`) with duplicate and leading-space filtering.
- **Flexible History Scoping Flags:** Added `--repo-history` (deterministic per-repository history files), `--history-file <PATH>` (explicit custom history path), and `--no-history` (for read-only filesystems or ephemeral sessions).
- **REPL History Visibility:** Displayed the active history path (or `disabled` status) in the interactive startup banner and added the `:history-path` (aliases `:history-file`, `:hp`) session command.

## 0.8.1

- set MSRV to `1.95`, thanks @ital0sant0s for reporting the build failure on older Rust due to the use of `cfg_select!`.

## 0.8.0

### Added
- **Git History & Commit Intent Search (`gix`):** Added `GitHistoryIndex` (`src/git.rs`) using pure-Rust `gitoxide` (`gix`) to index commit messages, authors, timestamps, and parent-to-child tree path changes into an in-memory Tantivy index.
- **`history` Subcommand & `--json` Support:** Added `blite history "<query>"` (`HistoryCommand`) to search commit intent and touched file paths (`path:manifests/...`) with structured `CommitHit` JSON output or formatted terminal summaries.
- **Lazy REPL `:history` Command:** Added `:history <query>` (aliases `:git`, `:log`) to interactive mode, lazily indexing Git history on first query and invalidating cleanly on `:reload`.

## 0.7.0

### Added
- **Kitchen vs. Cluster Drift Inspector (`KitchenComparator`):** Added `src/drift.rs` using `similar` to compare matched cluster files against their counterpart `RepoRelativePath` in a reference kitchen repository (`--kitchen <PATH>`), classifying files as `in_sync`, `drifted`, or `cluster_only`.
- **Unified Drift Diffs & Status Badges:** Attached `DriftReport` to `SearchHit` (`--json` compatible) and rendered color-coded `[in sync]`, `[drifted]`, and `[cluster-only]` badges alongside inline unified diffs (`↳ Kitchen Drift Diff:`) in terminal output.
- **Global `--kitchen` Flag:** Added `--kitchen` across single-shot `search`, `repl`, and default interactive mode so drift inspection works both before and after subcommands.

## 0.6.0

### Added
- **AST Function & Script Indexing (`tree-sitter`):** Added `chunk_code_file` (`src/code.rs`) using `tree-sitter-rust`, `tree-sitter-ruby`, and `tree-sitter-bash` (plus function/def block chunking for `.awk`, `.jq`, and `.yq` scripts and shebang detection) to split files over 80 lines into top-level definitions with leading doc comments attached.
- **Structured `symbol:` Queries & Line Ranges:** Added `SymbolName` (`nutype`) and `LineRange` to `SearchHit`, indexed `symbol`, `line_start`, and `line_end` in Tantivy, and enabled `symbol:<name>` query filtering.
- **Line-Accurate Terminal Rendering:** Updated `TerminalRenderer` to display `path:start..end` locations, `[fn/def <symbol>]` badges, and accurate source-file line numbers in syntax-highlighted snippets.

## 0.5.0

### Added
- **Auto-Discovery of Flux & Repo ConfigMaps (`SubstitutionTable`):** Automatically indexes `data` and `stringData` across repository `ConfigMap` manifests during parallel ingestion, prioritizing `flux-system/substitutions` (`manifests/flux-system/resources/ConfigMap/substitutions.yaml`) with fallback to other repository ConfigMaps.
- **Flux `envsubst` Expression Resolution:** Added pure-Rust `drone/envsubst`-compatible pattern matching (`${VAR}`, `${VAR:-default}`, `${VAR:=default}`, `${VAR:+alt}`, etc.) via `regex-lite` to resolve variable references inside retrieved manifests.
- **Substitutions Footer & Structured JSON Output:** Attached `resolved_vars` (`ResolvedVar` + `VarResolutionSource`) to `SearchHit` and rendered a color-coded `↳ Substitutions:` summary beneath matching manifests (explicitly flagging `<unresolved>` variables).

## 0.4.0

### Added
- **Kubernetes Manifest Metadata Extractor (`serde-saphyr` + `rayon`):** Added concurrent YAML parsing during repository indexing to extract `kind`, `metadata.name`, `metadata.namespace`, and `metadata.labels` into validated `nutype` domain types (`K8sKind`, `K8sName`, `K8sNamespace`).
- **Structured Query Filtering:** Indexed extracted Kubernetes fields in Tantivy so queries support field-prefixed filters (`kind:Deployment`, `ns:monitoring`, `name:api`, `labels:app=web`) alongside free-text search.
- **Resource Badges in Terminal & JSON Output:** Attached optional `K8sMetadata` to `SearchHit` and rendered `[Kind/name @ namespace]` badges in terminal headers.

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
