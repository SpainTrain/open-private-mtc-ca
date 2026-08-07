//! Clap v4 (derive) CLI surface for `mtcctl` (spec §17.3).
//!
//! Declares the complete subcommand tree from the spec diagram
//! (`docs/mtc-architecture-spec.md` §17.3) so it can be checked against the
//! spec by inspection, one screen at a time. [`crate::run`] is what
//! resolves each leaf to a handler (today: [`Command::Status`] is the only
//! one wired to the admin API; every other leaf resolves to
//! [`crate::error::CliError::NotImplemented`] -- individual operations are
//! separate, per-operation tickets, per ticket mtc-no9's Out of Scope).

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Admin CLI for the MTC Certificate Authority (spec §17).
#[derive(Debug, Parser)]
#[command(name = "mtcctl", version)]
pub struct Cli {
    /// Flags available on every subcommand.
    #[command(flatten)]
    pub global: GlobalArgs,

    /// The invoked subcommand.
    #[command(subcommand)]
    pub command: Command,
}

/// Flags available on every subcommand (spec §17.3: "Output formats",
/// "Authentication", "Authorization").
#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// Output format: human-readable tables, or machine-readable JSON/YAML.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub output: OutputFormat,

    /// Base URL of the admin API (spec §17.2).
    #[arg(long, global = true, default_value = "http://localhost:8080")]
    pub endpoint: String,

    /// Skip interactive confirmation for privileged commands (automation).
    ///
    /// Plumbed through per ticket mtc-no9's AC; the confirmation flow
    /// itself (interactive prompt, which commands count as privileged) is
    /// ticket adm-privileged-confirm.
    #[arg(long, global = true)]
    pub yes: bool,

    /// Explicitly confirm a privileged command.
    ///
    /// The non-interactive, single-command equivalent of `--yes` (spec
    /// §17.3: "privileged commands require explicit `--confirm` or
    /// interactive confirmation (skippable with `--yes` for automation)").
    /// See `--yes` above re: enforcement ticket.
    #[arg(long, global = true)]
    pub confirm: bool,
}

impl GlobalArgs {
    /// Whether the user has, by either flag, confirmed a privileged
    /// operation without needing an interactive prompt.
    #[must_use]
    pub const fn confirmed(&self) -> bool {
        self.yes || self.confirm
    }
}

/// Output format (spec §17.3): human-readable by default; `json` for agents
/// and scripts; `yaml` for config-style consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable tables (the default).
    #[default]
    Human,
    /// Machine-readable JSON.
    Json,
    /// Machine-readable YAML.
    Yaml,
}

/// Top-level subcommand tree (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show service status, lease, checkpoint.
    Status,

    /// Log inspection and proof operations.
    #[command(subcommand)]
    Log(LogCommand),

    /// Certificate operations.
    #[command(subcommand)]
    Cert(CertCommand),

    /// Batch operations.
    #[command(subcommand)]
    Batch(BatchCommand),

    /// Write-lease operations.
    #[command(subcommand)]
    Lease(LeaseCommand),

    /// Multi-region failover operations.
    #[command(subcommand)]
    Failover(FailoverCommand),

    /// Revocation list operations.
    #[command(subcommand)]
    Revocation(RevocationCommand),

    /// Pruning operations.
    #[command(subcommand)]
    Prune(PruneCommand),

    /// Self-auditor operations.
    #[command(subcommand)]
    Audit(AuditCommand),

    /// Compliance and operational reports.
    #[command(subcommand)]
    Report(ReportCommand),

    /// `EntryIntake` adapter operations.
    #[command(subcommand)]
    Adapter(AdapterCommand),

    /// Interactive REPL mode (ticket adm-repl; out of scope here).
    Repl,

    /// Generate shell completion scripts (ticket adm-completion; out of
    /// scope here).
    Completion,
}

/// `mtcctl log` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum LogCommand {
    /// Show log state (size, recent batches, etc.).
    Inspect,
    /// Generate inclusion proof for an index.
    Inclusion,
    /// Generate consistency proof between sizes.
    Consistency,
    /// Verify a certificate against the log.
    Verify,
}

/// `mtcctl cert` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum CertCommand {
    /// Issue a cert via ACME (for testing).
    Issue,
    /// Forensics: full info on a cert by index.
    Lookup,
    /// Revoke a cert (privileged).
    Revoke,
}

/// `mtcctl batch` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum BatchCommand {
    /// List recent batches.
    List,
    /// Show full batch details.
    Inspect,
}

/// `mtcctl lease` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum LeaseCommand {
    /// Current lease holder, expiry.
    Show,
    /// Force renewal (dev/test only).
    Renew,
}

/// `mtcctl failover` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum FailoverCommand {
    /// Failover readiness assessment.
    Status,
    /// Initiate manual failover (privileged).
    Initiate,
}

/// `mtcctl revocation` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum RevocationCommand {
    /// Show current revocation list.
    List,
    /// Add range to revocation list (privileged).
    Add,
    /// Trigger emergency redistribution.
    Distribute,
}

/// `mtcctl prune` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum PruneCommand {
    /// Pruning watermark and pending work.
    Status,
    /// Trigger pruning workflow.
    Run,
}

/// `mtcctl audit` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum AuditCommand {
    /// Trigger self-auditor on demand.
    Run,
    /// Show audit history.
    History,
    /// Independently verify log consistency.
    Verify,
}

/// `mtcctl report` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum ReportCommand {
    /// Issuance log report.
    Issuance,
    /// Revocation report.
    Revocation,
    /// Compliance bundle.
    Compliance,
}

/// `mtcctl adapter` (spec §17.3).
#[derive(Debug, Subcommand)]
pub enum AdapterCommand {
    /// List configured adapters.
    List,
    /// Per-adapter health and intake rate.
    Status,
    /// Pause an adapter (privileged).
    Pause,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        // Catches derive-macro mistakes clap can only detect at runtime
        // (duplicate flags, conflicting args, ambiguous names, etc.).
        Cli::command().debug_assert();
    }

    #[test]
    fn default_output_is_human() {
        assert_eq!(OutputFormat::default(), OutputFormat::Human);
    }

    #[test]
    fn confirmed_is_true_when_either_flag_is_set() {
        let neither = GlobalArgs {
            output: OutputFormat::Human,
            endpoint: "http://localhost:8080".to_string(),
            yes: false,
            confirm: false,
        };
        assert!(!neither.confirmed());

        let yes_only = GlobalArgs {
            yes: true,
            ..neither.clone()
        };
        assert!(yes_only.confirmed());

        let confirm_only = GlobalArgs {
            confirm: true,
            ..neither
        };
        assert!(confirm_only.confirmed());
    }
}
