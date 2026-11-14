use camino::Utf8PathBuf;
use similar::TextDiff;

use crate::types::DriftReport;
use crate::types::DriftStatus;
use crate::types::RepoRelativePath;

#[derive(Debug, Clone)]
pub struct KitchenComparator {
    cluster_root: Utf8PathBuf,
    kitchen_root: Utf8PathBuf,
}

impl KitchenComparator {
    pub fn new(cluster_root: impl Into<Utf8PathBuf>, kitchen_root: impl Into<Utf8PathBuf>) -> Self {
        Self {
            cluster_root: cluster_root.into(),
            kitchen_root: kitchen_root.into(),
        }
    }

    /// Compares a cluster repository file against its counterpart at the same
    /// `RepoRelativePath` in the kitchen-sink repository.
    pub fn compare(&self, rel_path: &RepoRelativePath) -> DriftReport {
        let rel_str = rel_path.as_str().trim_start_matches("./").trim_start_matches('/');
        let kitchen_file = self.kitchen_root.join(rel_str);
        let cluster_file = self.cluster_root.join(rel_str);

        let Ok(kitchen_content) = fs_err::read_to_string(&kitchen_file) else {
            return DriftReport {
                status: DriftStatus::ClusterOnly,
                unified_diff: None,
            };
        };

        let Ok(cluster_content) = fs_err::read_to_string(&cluster_file) else {
            return DriftReport {
                status: DriftStatus::ClusterOnly,
                unified_diff: None,
            };
        };

        if kitchen_content == cluster_content {
            return DriftReport {
                status: DriftStatus::InSync,
                unified_diff: None,
            };
        }

        let diff = TextDiff::from_lines(&kitchen_content, &cluster_content)
            .unified_diff()
            .context_radius(2)
            .header(&format!("kitchen/{}", rel_path), &format!("cluster/{}", rel_path))
            .to_string();

        DriftReport {
            status: DriftStatus::Drifted,
            unified_diff: Some(diff),
        }
    }
}
