// src/embed/error.rs

use camino::Utf8PathBuf;
use miette::Diagnostic;
use thiserror::Error;

use crate::config::ConfigError;

#[derive(Debug, Error, Diagnostic)]
pub enum EmbedError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),

    #[error("failed to read model file")]
    #[diagnostic(code(blite::embed::io))]
    Io(#[from] std::io::Error),

    #[error("unknown model '{0}'; run `blite models list` to inspect available models")]
    #[diagnostic(code(blite::embed::unknown_model))]
    UnknownModel(String),

    #[error("model directory '{path}' is missing required file '{missing}'")]
    #[diagnostic(
        code(blite::embed::incomplete_model_dir),
        help("A valid model directory must contain config.json, tokenizer.json, and model.safetensors.")
    )]
    IncompleteModelDir { path: Utf8PathBuf, missing: String },

    #[error("semantic search was explicitly requested, but no embedding model is configured or installed")]
    #[diagnostic(
        code(blite::embed::model_unavailable),
        help(
            "Run `blite models list` and `blite models download <model-id>` (or pass `--model-dir <path>`), or omit `--semantic` to fall back to lexical search."
        )
    )]
    ModelUnavailable,

    #[error("failed to parse model configuration JSON: {0}")]
    #[diagnostic(code(blite::embed::model_config))]
    ModelConfig(#[from] serde_json::Error),

    #[error("tokenizer error: {0}")]
    #[diagnostic(code(blite::embed::tokenizer))]
    Tokenizer(String),

    #[error("candle tensor/model execution error: {0}")]
    #[diagnostic(code(blite::embed::candle))]
    Candle(#[from] candle_core::Error),

    #[error("huggingface hub download error: {0}")]
    #[diagnostic(code(blite::embed::hf_hub))]
    HfHub(String),
}
