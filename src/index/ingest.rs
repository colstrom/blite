// src/index/ingest.rs

use std::sync::Mutex;

use camino::Utf8Path;
use ignore::WalkBuilder;
use rayon::prelude::*;

use super::IndexError;
use crate::code::chunk_code_file;
use crate::config::AppConfig;
use crate::git::collect_tracked_paths;
use crate::k8s::ParsedYaml;
use crate::k8s::parse_k8s_yaml_documents;
use crate::types::LineRange;
use crate::types::RepoRelativePath;
use crate::types::SymbolName;

#[derive(Debug, Clone)]
pub(super) struct IndexDocument {
    pub rel_path: RepoRelativePath,
    pub content: String,
    pub parsed_yaml: Option<ParsedYaml>,
    pub symbol: Option<SymbolName>,
    pub lines: Option<LineRange>,
    pub is_multi_doc: bool,
    pub doc_index: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct EmbeddableDoc {
    pub label: String,
    pub text: String,
    pub is_multi_doc: bool,
}

pub(super) fn format_doc_for_embedding(item: &IndexDocument) -> String {
    if let Some(parsed) = &item.parsed_yaml {
        let k8s = &parsed.metadata;
        let name = k8s.name.as_deref().map_or("unnamed", |v| v);
        let ns = k8s.namespace.as_deref().map_or("cluster-scoped", |v| v);
        return format!("{} {} in {} ({}):\n{}", k8s.kind, name, ns, item.rel_path, item.content);
    }
    if let Some(sym) = &item.symbol {
        return format!("{} ({}):\n{}", sym, item.rel_path, item.content);
    }
    format!("{}:\n{}", item.rel_path, item.content)
}

pub(super) fn collect_repo_documents(
    root: &Utf8Path,
    include_vendor: bool,
    include_untracked: bool,
) -> Result<Vec<IndexDocument>, IndexError> {
    let tracked_paths = if include_untracked { None } else { collect_tracked_paths(root) };

    let raw_files = Mutex::new(Vec::new());
    let mut builder = WalkBuilder::new(root.as_std_path());
    builder.hidden(false).git_ignore(true).add_custom_ignore_filename(".bliteignore");

    if let Ok(cfg_dir) = AppConfig::config_dir() {
        let global_ignore = cfg_dir.join("ignore");
        if global_ignore.is_file() {
            let _ = builder.add_ignore(global_ignore.as_std_path());
        }
    }

    builder.build_parallel().run(|| {
        Box::new(|entry| {
            let Ok(entry) = entry else {
                return ignore::WalkState::Continue;
            };

            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return ignore::WalkState::Continue;
            }

            let Some(utf8_path) = Utf8Path::from_path(entry.path()) else {
                return ignore::WalkState::Continue;
            };

            let rel = utf8_path.strip_prefix(root).unwrap_or(utf8_path).to_path_buf();

            let is_ignored_dir = rel.components().any(|c| {
                let name = c.as_str();
                name == ".git" || (!include_vendor && name == "vendor")
            });
            if is_ignored_dir {
                return ignore::WalkState::Continue;
            }

            let rel_path = RepoRelativePath::from(rel);
            if let Some(tracked) = &tracked_paths
                && !tracked.contains(&rel_path)
            {
                return ignore::WalkState::Continue;
            }

            if let Ok(content) = fs_err::read_to_string(utf8_path)
                && let Ok(mut guard) = raw_files.lock()
            {
                guard.push((rel_path, content));
            }

            ignore::WalkState::Continue
        })
    });

    let docs = raw_files
        .into_inner()
        .unwrap_or_default()
        .into_par_iter()
        .flat_map(|(rel_path, content)| {
            let yaml_docs = parse_k8s_yaml_documents(&rel_path, &content);
            if !yaml_docs.is_empty() {
                return yaml_docs
                    .into_iter()
                    .map(|yd| IndexDocument {
                        rel_path: rel_path.clone(),
                        content: yd.content,
                        parsed_yaml: Some(yd.parsed),
                        symbol: None,
                        lines: yd.lines,
                        is_multi_doc: yd.is_multi_doc,
                        doc_index: yd.is_multi_doc.then_some(yd.doc_index),
                    })
                    .collect();
            }

            let chunks = chunk_code_file(&rel_path, &content);
            if chunks.is_empty() {
                vec![IndexDocument {
                    rel_path,
                    content,
                    parsed_yaml: None,
                    symbol: None,
                    lines: None,
                    is_multi_doc: false,
                    doc_index: None,
                }]
            } else {
                chunks
                    .into_iter()
                    .map(|chunk| IndexDocument {
                        rel_path: rel_path.clone(),
                        content: chunk.content,
                        parsed_yaml: None,
                        symbol: chunk.symbol,
                        lines: Some(chunk.lines),
                        is_multi_doc: false,
                        doc_index: None,
                    })
                    .collect()
            }
        })
        .collect();
    Ok(docs)
}

pub fn collect_embeddable_docs(
    root: impl AsRef<Utf8Path>,
    include_vendor: bool,
    include_untracked: bool,
) -> Result<Vec<EmbeddableDoc>, IndexError> {
    let docs = collect_repo_documents(root.as_ref(), include_vendor, include_untracked)?;
    Ok(docs
        .iter()
        .map(|d| {
            let label = match d.lines {
                Some(r) => format!("{}:{}..{}", d.rel_path, r.start, r.end),
                None => d.rel_path.to_string(),
            };
            EmbeddableDoc {
                label,
                text: format_doc_for_embedding(d),
                is_multi_doc: d.is_multi_doc,
            }
        })
        .collect())
}
