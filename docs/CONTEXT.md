# Project Context & Architectural Brief

## 1. Problem Statement & Core Goal
Build a **completely self-contained, single-binary Rust CLI/TUI tool** that indexes a local Git repository and allows engineers to interactively explore and ask questions about its contents (Kubernetes manifests, internal tooling scripts, and documentation).

The tool is designed around a **progressive capability philosophy**:
1. **Zero-LLM Core (Phases 1–3):** Ultra-fast (<150ms indexing, <50ms query), deterministic, offline lexical + structural search with rich terminal formatting. Useful immediately with zero ML/GPU overhead.
2. **Semantic & Optional LLM Synthesis (Phase 4):** Adds local neural embeddings for conceptual search and a pluggable 3-tier LLM layer for conversational synthesis when desired.

---

## 2. Target Repository Profile & Domain Model
Understanding the structure of the primary target repository is critical for indexing, chunking, and ranking decisions:

* **Scale & File Distribution:**
  * ~2,500 files total (~1.5–2.5 MB raw text).
  * **~90% are YAML files**, almost all **<50 lines and <1 KB** (70% are <500 bytes), representing **one Kubernetes resource per file**.
  * *Architectural Consequence:* Because files are small single-resource manifests, **the whole file is already an optimal semantic chunk** for YAML. Complex sliding-window chunkers are unnecessary for manifests, and the entire repo can be indexed **in-memory (`tantivy::Index::create_in_ram()`)** on startup without writing cache files to disk.
* **Secondary File Types:**
  * Internal tooling written primarily in **Rust** and **Ruby**.
  * A notable volume of **`awk`, `yq`, and `jq` scripts** (often embedded in shell scripts or standalone files).
* **Multi-Repo "Kitchen" Topology:**
  * **Kitchen Repo:** A monorepo containing all tested, integrated, deployable capabilities ("kitchen").
  * **Cluster Repos:** Separate repositories containing a subset of the kitchen capabilities tailored to specific clusters.
  * **Git Relationship:** Cluster repos are **intentionally not Git forks** and **do not share a commit history/tree** with the kitchen repo. Changes are tested in kitchen and ported to clusters via remote diff/apply workflows (`git diff | git apply`).
  * **Layout & Customization Rules:**
    * Cluster repos share the **exact same relative filesystem paths** as the kitchen repo for common components.
    * Cluster-specific variations are isolated into separate files whenever possible.
    * Where variations cannot be isolated by file (e.g., cluster domains, environment-specific endpoints), values are substituted via **Kubernetes `ConfigMap` variables**.

---

## 3. Target Platforms & Deployment Constraints
* **Primary Developer/User Hardware:** Apple Silicon macOS (M-series). Needs native **Apple Metal** acceleration when local neural inference is enabled.
* **Secondary Target:** **Linux** (x86_64 and aarch64), primarily CPU-only environments. Must compile cleanly without OpenSSL dynamic linking issues (prefer `rustls-tls` over native OpenSSL so static `musl` builds remain viable).
* **Explicit Non-Target:** **Windows is out of scope.** Do not add Windows-specific path handling, conditional compilation, or cross-platform workarounds.
* **Binary Distribution:** Single standalone executable. No runtime dependencies on Python, Docker, Ollama, or external vector databases.

---

## 4. UX & Interaction Requirements
* **Dual-Mode Operation (Required from Phase 1):**
  * **Non-Interactive / Scriptable:** `tool "query"` supporting `--json` output for piping into `jq`, editors, or automation scripts.
  * **Interactive Exploration:** Invoking `tool` with no query launches a `rustyline` REPL (`> `) keeping the RAM index warm across queries.
* **Evolution to Full TUI (Phase 5):**
  * The core search/inference engine must be decoupled from stdout/stdin (via channels or clean service traits) so the Phase 1 REPL can later be replaced by a split-pane `ratatui` dashboard without rewriting retrieval logic.

---

## 5. Three-Tier LLM Inference Strategy (Phase 4)
All generative LLM features must sit behind a unified `LlmBackend` trait supporting three execution tiers:
1. **Tier 1 — Embedded Micro-Model (Airgapped Offline Default):** A small GGUF model (e.g., `Qwen2.5-Coder-0.5B` or `SmolLM2-360M`, ~300–400 MB) compiled into the binary via `include_bytes!` behind a Cargo feature flag (`--features embedded-llm`) so normal `cargo build` cycles stay fast and small (~15 MB).
2. **Tier 2 — Auto-Downloaded Local Model (Smart Offline):** When network and disk writes (`~/.cache/`) are permitted, download a more capable GGUF model (e.g., `Qwen2.5-Coder-7B` or `Llama-3.2-3B`) via `hf-hub` and run natively via `candle` (Metal on macOS, CPU on Linux).
3. **Tier 3 — Local Inference Server API:** Connect via HTTP/SSE (`reqwest`) to an OpenAI-compatible local server (e.g., Ollama, LM Studio, `llama-server`) for heavy models without increasing binary size.

*(Note: The small ~25 MB neural embedding model for Phase 4 semantic search should be unconditionally embedded via `include_bytes!` once introduced, as single-pass embedding is fast even on Linux CPUs.)*

---

## 6. Preferred Crate Ecosystem & Coding Conventions
When adding dependencies or writing modules, adhere to the project's established crate preferences:

* **Error Handling & Diagnostics:**
  * Use `thiserror` for structured domain/library errors.
  * Use `miette` (with `fancy` feature) for top-level CLI error reporting and rich source-span diagnostics (e.g., highlighting invalid YAML or unresolved ConfigMap keys).
  * Use `fs-err` instead of `std::fs` so all filesystem errors automatically include the offending file path.
* **Observability:**
  * Use `tracing` and `tracing-subscriber` (with `EnvFilter`) for structured logging. Keep stdout clean for query output/JSON; send logs to stderr.
* **Domain Modeling & Ergonomics:**
  * Use `nutype` to enforce validated newtypes (e.g., `RepoRelativePath`, `K8sKind`, `K8sNamespace`, `ConfigMapKey`) so paths from the kitchen repo and cluster repo cannot be accidentally conflated.
  * Use `bon` for builder patterns on search queries, indexer configurations, and LLM backend options.
  * Use `rayon` alongside `ignore::WalkParallel` for concurrent file ingestion and YAML/AST parsing.
  * Use `itertools` and `tap` for clean iterator pipelines and score-fusion logic.
* **Parsing & Search Stack:**
  * **YAML:** `serde-saphyr` (preferred over `serde_yaml` / `serde_yaml_ng`).
  * **Lexical Search:** `tantivy` (in-memory index for v1).
  * **Code AST:** `tree-sitter` (`tree-sitter-rust`, `tree-sitter-ruby`, `tree-sitter-bash`).
  * **Git & Diffing:** `gix` (`gitoxide`) for pure-Rust Git history/object access; `similar` for comparing kitchen vs. cluster files.
  * **CLI & Terminal Rendering:** `clap` (derive), `rustyline`, `syntect`, `termimad`, and later `ratatui` + `crossterm`.

---

## 7. Key Architectural Guardrails for Future Contributors / Agents
1. **Keep stdout sacred:** In single-shot mode (especially `--json`), never print status messages, progress bars, or logs to `stdout`. Route all `tracing` output and progress indicators to `stderr`.
2. **Preserve `RepoRelativePath` normalization:** Because Kitchen vs. Cluster comparison (Milestone 7) relies on matching relative paths across unrelated Git trees, normalize all indexed paths relative to the repository root immediately upon ingestion.
3. **Avoid C++ build friction where possible:** Prefer pure-Rust crates (`tantivy`, `serde-saphyr`, `gix`, `candle`, `rustls`) over C/C++ FFI wrappers (`libgit2`, `openssl`, `llama.cpp`) to guarantee effortless static builds across macOS and Linux. (`tree-sitter` C grammars are an accepted exception as they compile cleanly via `cc` with zero external system libs.)
