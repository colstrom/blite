# Phase 3: "Kitchen" & Git Awareness
**Goal:** Tailor the tool directly to the kitchen vs. cluster-subset repository model and Git commit history.

**Phase Crates:** `gix`, `similar`

- [ ] **Milestone 7: Kitchen vs. Cluster Drift Inspector (`--kitchen`)**
  - **Scope:** Add an optional `--kitchen <PATH>` flag. Because cluster repos share `RepoRelativePath` layouts without sharing Git history, compare matching paths using `similar` to flag whether a manifest is identical to kitchen, customized locally, or missing an upstream capability.
  - **Win:** Instant visibility into cluster drift and un-ported kitchen features without manual `git diff` plumbing.

- [ ] **Milestone 8: Git History & Commit Intent Search (`gix`)**
  - **Scope:** Use `gix` to index commit messages, authors, and touched `RepoRelativePath` entries from `.git`. Add a `--history` mode to surface the commit message and patch context explaining *why* a manifest or script was changed.
  - **Win:** Captures institutional memory and change intent directly from Git history.
