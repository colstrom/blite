// src/render/models.rs

use crate::command::models::ModelsPayload;

pub fn render_models_table(payload: &ModelsPayload) -> String {
    let mut out = String::new();
    out.push_str(&format!("\x1b[2mModels directory:\x1b[0m {}\n", payload.models_dir));
    out.push_str(&format!(
        "\x1b[2mHF cache access: \x1b[0m {}\n",
        if payload.allow_hf_cache { "enabled" } else { "disabled (opt-in)" }
    ));
    if let Some(custom_path) = &payload.active_model_path {
        out.push_str(&format!("\x1b[1;32mActive custom model:\x1b[0m {custom_path}\n"));
    }
    out.push('\n');

    for m in &payload.models {
        let mut status_badges = String::new();
        if m.active {
            status_badges.push_str(" \x1b[1;32m[active]\x1b[0m");
        }
        if m.installed {
            status_badges.push_str(" \x1b[1;36m[installed]\x1b[0m");
        }

        out.push_str(&format!("\x1b[1;33m{}\x1b[0m \x1b[2m({})\x1b[0m{status_badges}\n", m.id, m.hf_repo));
        out.push_str(&format!(
            "  Origin: {} | License: {} | Size: {} ({}, {} layers, {}d, {} ctx)\n",
            m.origin, m.license, m.approx_size, m.params, m.layers, m.dimensions, m.max_context
        ));
        out.push_str(&format!("  Speed:  {}\n", m.speed_tier));
        out.push_str(&format!("  Notes:  {}\n", m.compliance_notes));
        out.push_str(&format!("  Shines: {}\n", m.best_for));
        if let Some(path) = &m.local_path {
            out.push_str(&format!("  Path:   \x1b[2m{path}\x1b[0m\n"));
        }
        out.push('\n');
    }

    out.push_str("\x1b[2mUse `blite models download <id>` to install & activate a model, or `blite models use <path>` for a custom directory.\x1b[0m\n");
    out
}
