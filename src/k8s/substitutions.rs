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

        let (value, source) = match (op, operand, lookup) {
            (Some(":+"), Some(alt), Some((val, source))) if !val.is_empty() => (Some(alt.to_string()), source),
            (Some("+"), Some(alt), Some((_, source))) => (Some(alt.to_string()), source),
            (Some(":-" | ":="), Some(default_val), None) => (Some(default_val.to_string()), VarResolutionSource::InlineDefault),
            (Some(":-" | ":="), Some(default_val), Some((val, _))) if val.is_empty() => {
                (Some(default_val.to_string()), VarResolutionSource::InlineDefault)
            }
            (Some("-" | "="), Some(default_val), None) => (Some(default_val.to_string()), VarResolutionSource::InlineDefault),
            (_, _, Some((val, source))) => (Some(val.to_string()), source),
            (_, _, None) => (None, VarResolutionSource::Unresolved),
        };

        ResolvedVar {
            expression: expression.to_string(),
            name: name.to_string(),
            value,
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use camino::Utf8Path;

    use super::SubstitutionTable;
    use crate::types::K8sKind;
    use crate::types::K8sMetadata;
    use crate::types::K8sName;
    use crate::types::K8sNamespace;
    use crate::types::ResolvedVar;
    use crate::types::VarResolutionSource;

    fn cm_meta(ns: Option<&str>, name: Option<&str>) -> K8sMetadata {
        K8sMetadata {
            kind: K8sKind::try_from("ConfigMap".to_string()).unwrap(),
            name: name.map(|n| K8sName::try_from(n.to_string()).unwrap()),
            namespace: ns.map(|n| K8sNamespace::try_from(n.to_string()).unwrap()),
            labels: Vec::new(),
            annotations: Vec::new(),
        }
    }

    #[test]
    fn prioritizes_flux_substitutions_over_other_configmaps() {
        let mut table = SubstitutionTable::default();

        table.insert_configmap(
            Utf8Path::new("apps/default/cm.yaml"),
            &cm_meta(Some("default"), Some("app-settings")),
            BTreeMap::from([
                ("CLUSTER_DOMAIN".to_string(), "fallback.local".to_string()),
                ("APP_ONLY".to_string(), "from-app".to_string()),
            ]),
        );

        table.insert_configmap(
            Utf8Path::new("clusters/prod/substitutions.yaml"),
            &cm_meta(Some("flux-system"), Some("substitutions")),
            BTreeMap::from([("CLUSTER_DOMAIN".to_string(), "example.com".to_string())]),
        );

        assert_eq!(table.flux_var_count(), 1);

        let resolved = table.resolve_in_content("host: ${CLUSTER_DOMAIN}, extra: ${APP_ONLY}");
        assert_eq!(
            resolved,
            vec![
                ResolvedVar {
                    expression: "${CLUSTER_DOMAIN}".to_string(),
                    name: "CLUSTER_DOMAIN".to_string(),
                    value: Some("example.com".to_string()),
                    source: VarResolutionSource::FluxSubstitutions,
                },
                ResolvedVar {
                    expression: "${APP_ONLY}".to_string(),
                    name: "APP_ONLY".to_string(),
                    value: Some("from-app".to_string()),
                    source: VarResolutionSource::OtherConfigMap {
                        namespace: Some("default".to_string()),
                        name: "app-settings".to_string(),
                    },
                },
            ]
        );
    }

    #[test]
    fn recognizes_flux_substitutions_by_canonical_path() {
        let mut table = SubstitutionTable::default();
        table.insert_configmap(
            Utf8Path::new("cluster/flux-system/resources/ConfigMap/substitutions.yaml"),
            &cm_meta(None, None),
            BTreeMap::from([("REGION".to_string(), "ca-west-1".to_string())]),
        );

        let resolved = table.resolve_in_content("region: ${REGION}");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].value.as_deref(), Some("ca-west-1"));
        assert_eq!(resolved[0].source, VarResolutionSource::FluxSubstitutions);
    }

    #[test]
    fn handles_envsubst_default_and_alternate_operators() {
        let mut table = SubstitutionTable::default();
        table.insert_configmap(
            Utf8Path::new("flux/subs.yaml"),
            &cm_meta(Some("flux-system"), Some("substitutions")),
            BTreeMap::from([
                ("SET_VAR".to_string(), "hello".to_string()),
                ("EMPTY_VAR".to_string(), String::new()),
            ]),
        );

        let manifest = "\
a: ${SET_VAR:-fallback}
b: ${EMPTY_VAR:-fallback}
c: ${EMPTY_VAR-fallback}
d: ${UNSET_VAR-fallback}
e: ${EMPTY_VAR:=assigned}
f: ${SET_VAR:+alt_non_empty}
g: ${EMPTY_VAR:+alt_non_empty}
h: ${EMPTY_VAR+alt_when_set}
i: ${UNSET_VAR+alt_when_set}
j: ${UNSET_VAR}
";

        let resolved = table.resolve_in_content(manifest);
        let summary: Vec<(&str, Option<&str>, &VarResolutionSource)> = resolved
            .iter()
            .map(|r| (r.expression.as_str(), r.value.as_deref(), &r.source))
            .collect();

        assert_eq!(
            summary,
            vec![
                ("${SET_VAR:-fallback}", Some("hello"), &VarResolutionSource::FluxSubstitutions),
                ("${EMPTY_VAR:-fallback}", Some("fallback"), &VarResolutionSource::InlineDefault),
                ("${EMPTY_VAR-fallback}", Some(""), &VarResolutionSource::FluxSubstitutions),
                ("${UNSET_VAR-fallback}", Some("fallback"), &VarResolutionSource::InlineDefault),
                ("${EMPTY_VAR:=assigned}", Some("assigned"), &VarResolutionSource::InlineDefault),
                (
                    "${SET_VAR:+alt_non_empty}",
                    Some("alt_non_empty"),
                    &VarResolutionSource::FluxSubstitutions
                ),
                ("${EMPTY_VAR:+alt_non_empty}", Some(""), &VarResolutionSource::FluxSubstitutions),
                (
                    "${EMPTY_VAR+alt_when_set}",
                    Some("alt_when_set"),
                    &VarResolutionSource::FluxSubstitutions
                ),
                ("${UNSET_VAR+alt_when_set}", None, &VarResolutionSource::Unresolved),
                ("${UNSET_VAR}", None, &VarResolutionSource::Unresolved),
            ]
        );
    }

    #[test]
    fn deduplicates_identical_expressions_while_preserving_order() {
        let mut table = SubstitutionTable::default();
        table.insert_configmap(
            Utf8Path::new("flux/subs.yaml"),
            &cm_meta(Some("flux-system"), Some("substitutions")),
            BTreeMap::from([("DOMAIN".to_string(), "example.org".to_string())]),
        );

        let content = "a: ${DOMAIN}, b: ${OTHER:-1}, c: ${DOMAIN}, d: ${DOMAIN:-fallback}";
        let resolved = table.resolve_in_content(content);
        let exprs: Vec<&str> = resolved.iter().map(|r| r.expression.as_str()).collect();

        assert_eq!(exprs, vec!["${DOMAIN}", "${OTHER:-1}", "${DOMAIN:-fallback}"]);
    }
}
