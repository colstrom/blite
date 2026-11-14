# Phase 4: Hybrid Semantic Search & 3-Tier LLM Synthesis
**Goal:** Add conceptual natural-language matching and an optional, pluggable 3-tier LLM synthesis layer.

**Phase Crates:** `candle-core`, `candle-nn`, `candle-transformers`, `tokenizers`, `instant-distance`, `hf-hub`, `reqwest`

- [ ] **Milestone 9: Embedded Neural Semantic Search (`candle` + `instant-distance`)**
  - **Scope:** Embed a tiny quantized embedding model (`all-MiniLM-L6-v2` or `bge-small-en-v1.5`, ~25 MB) via `include_bytes!`. Combine vector similarity with `tantivy` BM25 scores using Reciprocal Rank Fusion (RRF) via `itertools`.
  - **Win:** Conceptual queries (e.g., *"how is TLS cert rotation configured?"*) match `cert-manager` manifests even without exact keyword overlap.

- [ ] **Milestone 10: Local Inference Server API Backend (`--llm-api`)**
  - **Scope:** Define an async `LlmBackend` trait (configured with a `bon` builder) and implement an OpenAI-compatible HTTP streaming client (`reqwest` with `rustls-tls`) to synthesize retrieved YAML, ConfigMap, and Git context using a local server (Ollama/LM Studio/llama.cpp on Apple Silicon).
  - **Win:** Full conversational Q&A when running at a workstation with zero binary size increase.

- [ ] **Milestone 11: Embedded Micro-LLM & Auto-Download Tiers**
  - **Scope:** Add the `candle` (Metal on macOS / CPU on Linux) GGUF runner to support (a) `--features embedded-llm` (baking in a ~350 MB Qwen2.5-Coder-0.5B model for strict offline use) and (b) `--download-model` (fetching a smarter ~2–4 GB model via `hf-hub` to `~/.cache`).
  - **Win:** Completes the 3-tier self-contained inference architecture.
