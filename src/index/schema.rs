// src/index/schema.rs

use camino::Utf8PathBuf;
use tantivy::TantivyDocument;
use tantivy::schema::Field;
use tantivy::schema::INDEXED;
use tantivy::schema::STORED;
use tantivy::schema::Schema;
use tantivy::schema::TEXT;
use tantivy::schema::Value;

use super::SearchHit;
use super::ingest::IndexDocument;
use crate::k8s::SubstitutionTable;
use crate::types::K8sKind;
use crate::types::K8sMetadata;
use crate::types::K8sName;
use crate::types::K8sNamespace;
use crate::types::LineRange;
use crate::types::RepoRelativePath;
use crate::types::SymbolName;

pub(super) struct IndexSchema {
    pub doc_id: Field,
    pub path: Field,
    pub body: Field,
    pub kind: Field,
    pub name: Field,
    pub ns: Field,
    pub labels: Field,
    pub annotations: Field,
    pub symbol: Field,
    pub line_start: Field,
    pub line_end: Field,
}

impl IndexSchema {
    pub fn build() -> (Schema, Self) {
        let mut builder = Schema::builder();
        let fields = Self {
            doc_id: builder.add_u64_field("doc_id", INDEXED | STORED),
            path: builder.add_text_field("path", TEXT | STORED),
            body: builder.add_text_field("body", TEXT | STORED),
            kind: builder.add_text_field("kind", TEXT | STORED),
            name: builder.add_text_field("name", TEXT | STORED),
            ns: builder.add_text_field("ns", TEXT | STORED),
            labels: builder.add_text_field("labels", TEXT | STORED),
            annotations: builder.add_text_field("annotations", TEXT | STORED),
            symbol: builder.add_text_field("symbol", TEXT | STORED),
            line_start: builder.add_u64_field("line_start", STORED),
            line_end: builder.add_u64_field("line_end", STORED),
        };
        (builder.build(), fields)
    }

    pub fn searchable_fields(&self) -> Vec<Field> {
        vec![
            self.path,
            self.body,
            self.kind,
            self.name,
            self.ns,
            self.labels,
            self.annotations,
            self.symbol,
        ]
    }

    pub fn to_tantivy_doc(&self, doc_id: u64, item: &IndexDocument) -> TantivyDocument {
        let mut document = TantivyDocument::new();
        document.add_u64(self.doc_id, doc_id);
        document.add_text(self.path, item.rel_path.as_str());
        document.add_text(self.body, &item.content);

        if let Some(parsed) = &item.parsed_yaml {
            let k8s = &parsed.metadata;
            document.add_text(self.kind, &k8s.kind);
            if let Some(name) = &k8s.name {
                document.add_text(self.name, name);
            }
            if let Some(ns) = &k8s.namespace {
                document.add_text(self.ns, ns);
            }
            for label in &k8s.labels {
                document.add_text(self.labels, label);
            }
            for key in k8s.label_keys() {
                document.add_text(self.labels, key);
            }
            for annotation in &k8s.annotations {
                document.add_text(self.annotations, annotation);
            }
            for key in k8s.annotation_keys() {
                document.add_text(self.annotations, key);
            }
        }

        if let Some(symbol) = &item.symbol {
            document.add_text(self.symbol, symbol);
        }

        if let Some(lines) = item.lines {
            document.add_u64(self.line_start, lines.start as u64);
            document.add_u64(self.line_end, lines.end as u64);
        }

        document
    }

    pub fn hydrate_hit(&self, doc: &TantivyDocument, score: f32, substitutions: &SubstitutionTable) -> SearchHit {
        let path_str = doc.get_first(self.path).and_then(|v| v.as_str()).unwrap_or_default();
        let content = doc.get_first(self.body).and_then(|v| v.as_str()).unwrap_or_default().to_string();

        let k8s = self.extract_k8s(doc);
        let symbol = doc
            .get_first(self.symbol)
            .and_then(|v| v.as_str())
            .and_then(|s| SymbolName::try_from(s.to_string()).ok());
        let lines = self.extract_lines(doc);

        let resolved_vars = if k8s.is_some() {
            substitutions.resolve_in_content(&content)
        } else {
            Vec::new()
        };

        SearchHit {
            path: RepoRelativePath::from(Utf8PathBuf::from(path_str)),
            score,
            k8s,
            symbol,
            lines,
            resolved_vars,
            recent_commits: Vec::new(),
            content,
            drift: None,
        }
    }

    fn extract_k8s(&self, doc: &TantivyDocument) -> Option<K8sMetadata> {
        let kind_str = doc.get_first(self.kind).and_then(|v| v.as_str())?;
        let kind = K8sKind::try_from(kind_str).ok()?;

        let name = doc
            .get_first(self.name)
            .and_then(|v| v.as_str())
            .and_then(|s| K8sName::try_from(s).ok());

        let namespace = doc
            .get_first(self.ns)
            .and_then(|v| v.as_str())
            .and_then(|s| K8sNamespace::try_from(s).ok());

        let labels = doc
            .get_all(self.labels)
            .filter_map(|v| v.as_str())
            .filter(|s| s.contains('='))
            .map(ToString::to_string)
            .collect();

        let annotations = doc
            .get_all(self.annotations)
            .filter_map(|v| v.as_str())
            .filter(|s| s.contains('='))
            .map(ToString::to_string)
            .collect();

        Some(K8sMetadata {
            kind,
            name,
            namespace,
            labels,
            annotations,
        })
    }

    fn extract_lines(&self, doc: &TantivyDocument) -> Option<LineRange> {
        let start = doc.get_first(self.line_start).and_then(|v| v.as_u64())? as usize;
        let end = doc.get_first(self.line_end).and_then(|v| v.as_u64())? as usize;
        Some(LineRange { start, end })
    }
}
