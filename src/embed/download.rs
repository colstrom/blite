// src/embed/download.rs

use hf_hub::HFClientSync;

use super::EmbedError;
use super::ModelSpec;
use super::REQUIRED_MODEL_FILES;
use super::ValidatedModelDir;
use crate::config::AppConfig;

pub fn download_curated_model(spec: &ModelSpec) -> Result<ValidatedModelDir, EmbedError> {
    let dest_dir = AppConfig::models_dir()?.join(spec.id);
    fs_err::create_dir_all(&dest_dir)?;

    let (owner, name) = spec
        .hf_repo
        .split_once('/')
        .ok_or_else(|| EmbedError::HfHub(format!("invalid HuggingFace repo id '{}'", spec.hf_repo)))?;

    eprintln!("Downloading {} ({}) into {}...", spec.id, spec.hf_repo, dest_dir);

    let client = HFClientSync::new().map_err(|e| EmbedError::HfHub(e.to_string()))?;
    let repo = client.model(owner, name);

    for filename in REQUIRED_MODEL_FILES {
        repo.download_file()
            .filename(*filename)
            .local_dir(dest_dir.as_std_path())
            .send()
            .map_err(|e| EmbedError::HfHub(format!("failed to download '{filename}': {e}")))?;
    }

    let validated = ValidatedModelDir::try_from(dest_dir)?;
    eprintln!("\x1b[1;32mInstalled {}\x1b[0m at {}", spec.id, validated);
    Ok(validated)
}
