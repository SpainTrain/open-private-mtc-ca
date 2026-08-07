//! Output-format layer (spec §17.3 "Output formats").
//!
//! Ticket mtc-no9 AC: "Output layer: human tables, `--output json` and
//! `--output yaml` via serde on the same response types". Every command
//! renders the *same* generated-client response type three ways: a
//! human-readable [`comfy_table`], or `serde_json`/`serde_yaml` of that
//! identical value. There is no separate "display" DTO to keep in sync with
//! the wire schema -- the human renderer each command supplies reads
//! straight off the `mtc_admin_api_client::models` type.

use comfy_table::Table;
use serde::Serialize;

use crate::cli::OutputFormat;
use crate::error::CliError;

/// Renders `value` per `format`.
///
/// `human(value)` for [`OutputFormat::Human`], or `value` serialized via
/// serde for [`OutputFormat::Json`] / [`OutputFormat::Yaml`]. Pure (no I/O)
/// so output-format tests can assert on the result directly; [`emit`] is
/// the thin stdout-writing wrapper command handlers call.
///
/// # Errors
///
/// Returns [`CliError::Render`] if serde fails to serialize `value`.
pub fn render<T, F>(value: &T, format: OutputFormat, human: F) -> Result<String, CliError>
where
    T: Serialize,
    F: FnOnce(&T) -> String,
{
    match format {
        OutputFormat::Human => Ok(human(value)),
        OutputFormat::Json => {
            serde_json::to_string_pretty(value).map_err(|e| CliError::Render(e.to_string()))
        }
        OutputFormat::Yaml => {
            serde_yaml::to_string(value).map_err(|e| CliError::Render(e.to_string()))
        }
    }
}

/// Writes [`render`]'s output to stdout (ticket mtc-no9 AC: "stdout for
/// payload"; diagnostics/errors are the caller's job, via stderr).
///
/// # Errors
///
/// Returns [`CliError::Render`] if serde fails to serialize `value`.
pub fn emit<T, F>(value: &T, format: OutputFormat, human: F) -> Result<(), CliError>
where
    T: Serialize,
    F: FnOnce(&T) -> String,
{
    let rendered = render(value, format, human)?;
    println!("{rendered}");
    Ok(())
}

/// A blank two-column table pre-configured for `field | value` human output.
///
/// The shape every leaf command's human renderer starts from today.
/// List-shaped output (`batch list`, `revocation list`, etc.) is future,
/// per-operation-ticket work that will add its own header row via
/// [`comfy_table::Table`] directly.
#[must_use]
pub fn key_value_table() -> Table {
    let mut table = Table::new();
    table.load_style(comfy_table::presets::UTF8_FULL_CONDENSED);
    table.set_header(["field", "value"]);
    table
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde::Serialize;

    use super::{emit, render};
    use crate::cli::OutputFormat;

    #[derive(Serialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    fn sample() -> Sample {
        Sample {
            name: "widget".to_string(),
            count: 3,
        }
    }

    #[test]
    fn human_format_uses_the_supplied_renderer_verbatim() {
        let rendered = render(&sample(), OutputFormat::Human, |s| {
            format!("{}={}", s.name, s.count)
        })
        .expect("human render succeeds");
        assert_eq!(rendered, "widget=3");
    }

    #[test]
    fn json_format_serializes_the_value_directly() {
        let rendered =
            render(&sample(), OutputFormat::Json, |_| String::new()).expect("json render succeeds");
        let parsed: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!(parsed["name"], "widget");
        assert_eq!(parsed["count"], 3);
    }

    #[test]
    fn yaml_format_serializes_the_value_directly() {
        let rendered =
            render(&sample(), OutputFormat::Yaml, |_| String::new()).expect("yaml render succeeds");
        let parsed: serde_yaml::Value = serde_yaml::from_str(&rendered).expect("valid YAML");
        assert_eq!(parsed["name"], "widget");
        assert_eq!(parsed["count"], 3);
    }

    #[test]
    fn emit_does_not_error_for_any_format() {
        for format in [OutputFormat::Human, OutputFormat::Json, OutputFormat::Yaml] {
            emit(&sample(), format, |s| s.name.clone()).expect("emit succeeds");
        }
    }
}
