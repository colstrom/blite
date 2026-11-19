// src/k8s/substitutions.rs

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::OnceLock;

use camino::Utf8Path;
use regex_lite::Regex;

use crate::types::K8sMetadata;
use crate::types::ResolvedVar;
use crate::types::VarResolutionSource;

/// Holds substitution variables discovered across repository ConfigMaps,
/// prioritizing Flux's `flux-system/substitutions` ConfigMap.
#[derive(Debug, Clone, Default)]
pub struct SubstitutionTable {
    flux_vars: BTreeMap<String, String>,
    fallback_vars: BTreeMap<String, (String, Option<String>, String)>, // key -> (val, ns, cm_name)
}

impl SubstitutionTable {
    pub fn insert_configmap(&mut self, path: &Utf8Path, meta: &K8sMetadata, data: BTreeMap<String, String>) {
        let ns = meta.namespace.as_ref().map(|s| s.as_str());
        let name = meta.name.as_ref().map(|s| s.as_str());

        let is_flux_substitutions = (ns == Some("flux-system") && name == Some("substitutions"))
            || path.ends_with("flux-system/resources/ConfigMap/substitutions.yaml");

        if is_flux_substitutions {
            self.flux_vars.extend(data);
        } else {
            let cm_name = name.unwrap_or(path.as_str()).to_string();
            let ns_owned = ns.map(ToString::to_string);
            for (k, v) in data {
                self.fallback_vars
                    .entry(k)
                    .or_insert_with(|| (v, ns_owned.clone(), cm_name.clone()));
            }
        }
    }

    pub fn flux_var_count(&self) -> usize {
        self.flux_vars.len()
    }

    /// Scans `content` for `drone/envsubst`-style `${VAR}` / `${VAR:-default}` expressions
    /// and resolves them in order of appearance (deduplicated by raw expression).
    pub fn resolve_in_content(&self, content: &str) -> Vec<ResolvedVar> {
        static ENVSUBST_RE: OnceLock<Regex> = OnceLock::new();
        let re = ENVSUBST_RE.get_or_init(|| {
            Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?:(:?=|:=|:?-|-|:?\+|\+|:\?|\?)([^}]*))?\}").expect("valid envsubst regex")
        });

        let mut seen = HashSet::new();
        let mut resolved = Vec::new();

        for caps in re.captures_iter(content) {
            let full_expr = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
            if !seen.insert(full_expr.to_string()) {
                continue;
            }

            let var_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let op = caps.get(2).map(|m| m.as_str());
            let operand = caps.get(3).map(|m| m.as_str());

            resolved.push(self.resolve_one(full_expr, var_name, op, operand));
        }

        resolved
    }

    fn resolve_one(&self, expression: &str, name: &str, op: Option<&str>, operand: Option<&str>) -> ResolvedVar {
        let lookup = self
            .flux_vars
            .get(name)
            .map(|v| (v.as_str(), VarResolutionSource::FluxSubstitutions))
            .or_else(|| {
                self.fallback_vars.get(name).map(|(v, ns, cm_name)| {
                    (
                        v.as_str(),
                        VarResolutionSource::OtherConfigMap {
                            namespace: ns.clone(),
                            name: cm_name.clone(),
                        },
                    )
                })
            });

        let is_set = lookup.is_some();
        let is_non_empty = lookup.as_ref().is_some_and(|(v, _)| !v.is_empty());

        match (op, operand) {
            (Some(":+"), Some(alt)) if is_non_empty => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(alt.to_string()),
                source: lookup.unwrap().1,
            },
            (Some("+"), Some(alt)) if is_set => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(alt.to_string()),
                source: lookup.unwrap().1,
            },
            (Some(":-" | ":="), Some(default_val)) if !is_non_empty => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(default_val.to_string()),
                source: VarResolutionSource::InlineDefault,
            },
            (Some("-" | "="), Some(default_val)) if !is_set => ResolvedVar {
                expression: expression.to_string(),
                name: name.to_string(),
                value: Some(default_val.to_string()),
                source: VarResolutionSource::InlineDefault,
            },
            _ => match lookup {
                Some((val, source)) => ResolvedVar {
                    expression: expression.to_string(),
                    name: name.to_string(),
                    value: Some(val.to_string()),
                    source,
                },
                None => ResolvedVar {
                    expression: expression.to_string(),
                    name: name.to_string(),
                    value: None,
                    source: VarResolutionSource::Unresolved,
                },
            },
        }
    }
}
