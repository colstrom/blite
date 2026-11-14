# Phase 1: Instant In-Memory Search (Zero ML, Zero Disk Cache)
**Goal:** A working, ~12 MB static binary that indexes 2,500+ manifests in RAM and answers queries in milliseconds.

**Phase Crates:** `clap`, `ignore`, `tantivy`, `syntect`, `serde`, `serde_json`, `rustyline`, `thiserror`, `miette`, `fs-err`, `tracing`, `tracing-subscriber`, `bon`, `nutype`, `rayon`, `itertools`, `tap`

- [ ] **Milestone 1: Repo Walker, Domain Types & Single-Shot Lexical CLI**
  - **Scope:** Set up `tracing` + `miette` error reporting and `fs-err` for file reads. Define core `nutype` wrappers (`RepoRelativePath`, `SearchQuery`) and a `bon` builder for the indexer config. Walk the repo in parallel (`ignore::WalkParallel` + `rayon`), load files into `tantivy::Index::create_in_ram()`, and execute a single-shot query via `clap`.
  - **Win:** End-to-end working binary that proves sub-150ms in-memory indexing across all 2,500 files.

- [ ] **Milestone 2: Syntax-Highlighted Output & `--json` Mode**
  - **Scope:** Integrate `syntect` to render matching YAML, Rust, Ruby, and shell/awk snippets with line numbers and terminal colours. Add a `--json` flag (`serde_json`) emitting structured hit payloads for scripting.
  - **Win:** Immediately useful replacement for `grep`/`rg` when exploring manifests and scripts.

- [ ] **Milestone 3: Interactive REPL Mode**
  - **Scope:** When invoked with no positional query argument, drop into a `rustyline` loop (`> `) keeping the `tantivy` RAM index hot across queries. Support `:exit`, `:reload`, and command history.
  - **Win:** Dual-mode UX (scriptable single-shot CLI + exploratory REPL) complete.
