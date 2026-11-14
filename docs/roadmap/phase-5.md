# Phase 5: Terminal Dashboard
**Goal:** Graduate from a line-oriented REPL to a full-screen interactive explorer.

**Phase Crates:** `ratatui`, `crossterm`, `termimad`

- [ ] **Milestone 12: Split-Pane `ratatui` TUI**
  - **Scope:** Decouple the search/LLM engine over `tokio::sync::mpsc` channels and build a `ratatui` + `crossterm` interface with live query filtering, ranked K8s/code hits on the left, and a tabbed inspector on the right (Syntax-Highlighted Source, Resolved ConfigMaps, Kitchen Diff, and Git History).
  - **Win:** Polished, single-binary terminal workbench for exploring the entire platform.
