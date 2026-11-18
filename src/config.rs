use camino::Utf8Path;
use camino::Utf8PathBuf;
use miette::Diagnostic;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum ConfigError {
    #[error("failed to read or write configuration file")]
    #[diagnostic(code(blite::config::io))]
    Io(#[from] std::io::Error),

    #[error("failed to parse YAML configuration: {0}")]
    #[diagnostic(code(blite::config::yaml))]
    Yaml(#[from] serde_saphyr::DeserializeError),

    #[error("failed to serialize YAML configuration: {0}")]
    #[diagnostic(code(blite::config::yaml_ser))]
    YamlSerialize(#[from] serde_saphyr::SerializeError),

    #[error("failed to parse or serialize JSON configuration: {0}")]
    #[diagnostic(code(blite::config::json))]
    Json(#[from] serde_json::Error),

    #[error("failed to parse TOML configuration: {0}")]
    #[diagnostic(code(blite::config::toml_de))]
    TomlDe(#[from] toml::de::Error),

    #[error("failed to serialize TOML configuration: {0}")]
    #[diagnostic(code(blite::config::toml_ser))]
    TomlSer(#[from] toml::ser::Error),

    #[error("unable to determine platform configuration or data directory")]
    #[diagnostic(code(blite::config::no_dirs))]
    NoPlatformDirs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Yaml,
    Json,
    Toml,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    /// Curated model ID (e.g. `granite-30m`, `mxbai-xsmall`, `nomic-v1.5`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model: Option<String>,

    /// Explicit path to a user-provided local model directory
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_model_path: Option<Utf8PathBuf>,

    /// Explicit user consent to probe the local HuggingFace cache (`~/.cache/huggingface/hub`)
    #[serde(default)]
    pub allow_hf_cache: bool,
}

impl AppConfig {
    /// Returns `<config_dir>/blite`.
    pub fn config_dir() -> Result<Utf8PathBuf, ConfigError> {
        let base = dirs::config_dir().ok_or(ConfigError::NoPlatformDirs)?;
        Utf8PathBuf::try_from(base)
            .map(|dir| dir.join("blite"))
            .map_err(|_| ConfigError::NoPlatformDirs)
    }

    /// Returns `<data_local_dir>/blite/models`, `blite`'s app-owned model storage directory.
    pub fn models_dir() -> Result<Utf8PathBuf, ConfigError> {
        let base = dirs::data_local_dir().ok_or(ConfigError::NoPlatformDirs)?;
        Utf8PathBuf::try_from(base)
            .map(|dir| dir.join("blite").join("models"))
            .map_err(|_| ConfigError::NoPlatformDirs)
    }

    /// Discovers an existing config file in priority order:
    /// `config.yaml` -> `config.yml` -> `config.json` -> `config.toml`.
    pub fn discover_file() -> Result<Option<(Utf8PathBuf, ConfigFormat)>, ConfigError> {
        let dir = Self::config_dir()?;
        let candidates = [
            (dir.join("config.yaml"), ConfigFormat::Yaml),
            (dir.join("config.yml"), ConfigFormat::Yaml),
            (dir.join("config.json"), ConfigFormat::Json),
            (dir.join("config.toml"), ConfigFormat::Toml),
        ];

        for (path, format) in candidates {
            if path.exists() {
                return Ok(Some((path, format)));
            }
        }

        Ok(None)
    }

    /// Loads the first discovered config file (`YAML` -> `JSON` -> `TOML`),
    /// returning `AppConfig::default()` if none exists yet.
    pub fn load() -> Result<Self, ConfigError> {
        let Some((path, format)) = Self::discover_file()? else {
            return Ok(Self::default());
        };
        Self::load_from_path(&path, format)
    }

    pub fn load_from_path(path: &Utf8Path, format: ConfigFormat) -> Result<Self, ConfigError> {
        let raw = fs_err::read_to_string(path)?;
        let cfg = match format {
            ConfigFormat::Yaml => serde_saphyr::from_str(&raw)?,
            ConfigFormat::Json => serde_json::from_str(&raw)?,
            ConfigFormat::Toml => toml::from_str(&raw)?,
        };
        Ok(cfg)
    }

    /// Saves the configuration to the existing discovered file in its original format,
    /// or creates `<config_dir>/blite/config.yaml` by default.
    pub fn save(&self) -> Result<Utf8PathBuf, ConfigError> {
        let (path, format) = match Self::discover_file()? {
            Some(found) => found,
            None => (Self::config_dir()?.join("config.yaml"), ConfigFormat::Yaml),
        };

        if let Some(parent) = path.parent() {
            fs_err::create_dir_all(parent)?;
        }

        let serialized = match format {
            ConfigFormat::Yaml => serde_saphyr::to_string(self)?,
            ConfigFormat::Json => serde_json::to_string_pretty(self)?,
            ConfigFormat::Toml => toml::to_string_pretty(self)?,
        };

        fs_err::write(&path, serialized)?;
        Ok(path)
    }
}
