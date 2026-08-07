//! `mtcctl`: the admin CLI for the MTC Certificate Authority (spec §17.3).
//!
//! Declares the complete clap subcommand tree ([`cli`]) and dispatches it
//! ([`run`]) to per-operation handlers under [`commands`]. Only `status` is
//! wired to the admin API today; every other leaf resolves to a distinct,
//! non-zero-exit [`error::CliError::NotImplemented`] (ticket mtc-no9's Out
//! of Scope: individual operations are separate, per-operation tickets).
//! All API calls go through the generated `mtc_admin_api_client` (spec
//! §17.2) -- this crate never builds an HTTP request itself.

pub mod cli;
pub mod client;
pub mod commands;
pub mod error;
pub mod output;

use cli::{
    AdapterCommand, AuditCommand, BatchCommand, CertCommand, Cli, Command, FailoverCommand,
    LeaseCommand, LogCommand, PruneCommand, ReportCommand, RevocationCommand,
};
use error::CliError;

/// Dispatches a parsed [`Cli`] to its command handler.
///
/// # Errors
///
/// Returns [`error::CliError::NotImplemented`] for every leaf not yet wired
/// to the admin API, or whatever [`commands::status::run`] returns for
/// `status`.
pub async fn run(cli: &Cli) -> Result<(), CliError> {
    match &cli.command {
        Command::Status => commands::status::run(&cli.global).await,
        Command::Log(sub) => Err(commands::not_implemented(match sub {
            LogCommand::Inspect => "log inspect",
            LogCommand::Inclusion => "log inclusion",
            LogCommand::Consistency => "log consistency",
            LogCommand::Verify => "log verify",
        })),
        Command::Cert(sub) => Err(commands::not_implemented(match sub {
            CertCommand::Issue => "cert issue",
            CertCommand::Lookup => "cert lookup",
            CertCommand::Revoke => "cert revoke",
        })),
        Command::Batch(sub) => Err(commands::not_implemented(match sub {
            BatchCommand::List => "batch list",
            BatchCommand::Inspect => "batch inspect",
        })),
        Command::Lease(sub) => Err(commands::not_implemented(match sub {
            LeaseCommand::Show => "lease show",
            LeaseCommand::Renew => "lease renew",
        })),
        Command::Failover(sub) => Err(commands::not_implemented(match sub {
            FailoverCommand::Status => "failover status",
            FailoverCommand::Initiate => "failover initiate",
        })),
        Command::Revocation(sub) => Err(commands::not_implemented(match sub {
            RevocationCommand::List => "revocation list",
            RevocationCommand::Add => "revocation add",
            RevocationCommand::Distribute => "revocation distribute",
        })),
        Command::Prune(sub) => Err(commands::not_implemented(match sub {
            PruneCommand::Status => "prune status",
            PruneCommand::Run => "prune run",
        })),
        Command::Audit(sub) => Err(commands::not_implemented(match sub {
            AuditCommand::Run => "audit run",
            AuditCommand::History => "audit history",
            AuditCommand::Verify => "audit verify",
        })),
        Command::Report(sub) => Err(commands::not_implemented(match sub {
            ReportCommand::Issuance => "report issuance",
            ReportCommand::Revocation => "report revocation",
            ReportCommand::Compliance => "report compliance",
        })),
        Command::Adapter(sub) => Err(commands::not_implemented(match sub {
            AdapterCommand::List => "adapter list",
            AdapterCommand::Status => "adapter status",
            AdapterCommand::Pause => "adapter pause",
        })),
        Command::Repl => Err(commands::not_implemented("repl")),
        Command::Completion => Err(commands::not_implemented("completion")),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;

    use super::*;

    /// One dispatch check per leaf declared in the spec §17.3 tree (except
    /// `status`, covered by `commands::status`'s own tests against a live
    /// server): every leaf must be a distinct [`CliError::NotImplemented`]
    /// naming its own full command path, mapping to exit code `3`.
    #[tokio::test]
    async fn every_unimplemented_leaf_reports_its_own_path_and_exit_code_3() {
        let cases: &[&[&str]] = &[
            &["log", "inspect"],
            &["log", "inclusion"],
            &["log", "consistency"],
            &["log", "verify"],
            &["cert", "issue"],
            &["cert", "lookup"],
            &["cert", "revoke"],
            &["batch", "list"],
            &["batch", "inspect"],
            &["lease", "show"],
            &["lease", "renew"],
            &["failover", "status"],
            &["failover", "initiate"],
            &["revocation", "list"],
            &["revocation", "add"],
            &["revocation", "distribute"],
            &["prune", "status"],
            &["prune", "run"],
            &["audit", "run"],
            &["audit", "history"],
            &["audit", "verify"],
            &["report", "issuance"],
            &["report", "revocation"],
            &["report", "compliance"],
            &["adapter", "list"],
            &["adapter", "status"],
            &["adapter", "pause"],
            &["repl"],
            &["completion"],
        ];

        for args in cases {
            let mut full = vec!["mtcctl"];
            full.extend_from_slice(args);
            let cli = Cli::parse_from(&full);

            let err = run(&cli)
                .await
                .expect_err("unimplemented leaf must return Err");
            assert_eq!(err.exit_code(), 3, "leaf: {args:?}");

            let expected_path = args.join(" ");
            assert_eq!(
                err.to_string(),
                format!("{expected_path} is not yet implemented"),
                "leaf: {args:?}"
            );
        }
    }
}
