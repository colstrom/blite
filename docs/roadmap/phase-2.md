# Phase 2: Kubernetes & Code Structural Intelligence
**Goal:** Understand Kubernetes resources, ConfigMap substitutions, and internal tool code symbols rather than treating files as flat text.

**Phase Crates:** `serde-saphyr`, `tree-sitter`, `tree-sitter-rust`, `tree-sitter-ruby`, `tree-sitter-bash`

- [ ] **Milestone 4: Kubernetes Manifest Metadata Extractor**
  - **Scope:** Parse YAML files in parallel (`rayon`) using `serde-saphyr` to extract `kind`, `metadata.name`, `metadata.namespace`, and `metadata.labels` into dedicated `tantivy` schema fields. Support structured query filters (e.g., `kind:Deployment redis` or `ns:monitoring alertmanager`).
  - **Win:** High-precision filtering across 2,500 single-resource manifests.

- [ ] **Milestone 5: ConfigMap Variable & Reference Resolver**
  - **Scope:** Index `ConfigMap` keys (`data.*`) into an in-memory lookup map and scan manifests for `${VAR}`, `$(VAR)`, and `configMapKeyRef` references. When displaying a snippet, automatically render a resolved-values footer showing what each variable evaluates to in the repo's ConfigMaps.
  - **Win:** Solves cluster-specific variable/domain substitution indirection without needing an LLM.

- [ ] **Milestone 6: AST Function & Script Indexing (`tree-sitter`)**
  - **Scope:** Add `tree-sitter-rust`, `tree-sitter-ruby`, and `tree-sitter-bash` (plus lightweight block chunking for `awk`/`yq`/`jq` scripts) so queries against internal tooling return complete functions/methods with doc comments rather than isolated lines.
  - **Win:** Internal Rust, Ruby, and script tooling become first-class searchable citizens alongside YAML.
