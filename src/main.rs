// src/main.rs

use blite::Command;
use clap::Parser;
use miette::IntoDiagnostic;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

fn main() -> miette::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_writer(std::io::stderr)
                .flatten_event(true),
        )
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")))
        .init();

    Command::parse().run().into_diagnostic()
}
