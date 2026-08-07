//! `mtcctl` binary entry point (spec §17.3).
//!
//! Not the usual plain `fn main() -> eyre::Result<()>` shape (rule
//! `thiserror-for-libs-eyre-for-bins`'s default bin template): ticket
//! mtc-no9's AC requires distinct process exit codes per error kind, and
//! `eyre::Result`'s `Termination` impl can only ever produce exit code `0`
//! or `1`. `color-eyre` still installs the panic/error report hook here at
//! the top level; [`mtcctl::error::CliError::exit_code`] supplies the
//! per-command exit status (stdout carries the payload, stderr carries
//! diagnostics -- ticket mtc-no9 AC).

use std::process::ExitCode;

use clap::Parser;
use mtcctl::cli::Cli;

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = color_eyre::install() {
        eprintln!("mtcctl: failed to install error reporter: {err}");
    }

    // `Cli::parse()` prints usage/help and calls `std::process::exit(2)`
    // itself on a parse error -- clap owns exit code 2 (see error.rs).
    let cli = Cli::parse();

    match mtcctl::run(&cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.exit_code())
        }
    }
}
