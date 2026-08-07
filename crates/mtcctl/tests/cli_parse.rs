//! Clap argument-parsing tests over the full spec §17.3 subcommand tree
//! (ticket mtc-no9 Testing: "Unit: clap parse tests
//! (`Command::debug_assert`)").

use clap::{CommandFactory, Parser};
use mtcctl::cli::{Cli, Command, OutputFormat};

#[test]
fn cli_definition_is_valid() {
    // Catches derive-macro mistakes clap can only detect at runtime
    // (duplicate flags, conflicting args, ambiguous names, etc.) -- the
    // exact tool ticket mtc-no9's Testing section names.
    Cli::command().debug_assert();
}

#[test]
fn parses_bare_status_with_default_globals() {
    let cli = Cli::parse_from(["mtcctl", "status"]);
    assert!(matches!(cli.command, Command::Status));
    assert_eq!(cli.global.output, OutputFormat::Human);
    assert_eq!(cli.global.endpoint, "http://localhost:8080");
    assert!(!cli.global.yes);
    assert!(!cli.global.confirm);
}

#[test]
fn global_flags_parse_before_the_subcommand() {
    let cli = Cli::parse_from([
        "mtcctl",
        "--output",
        "json",
        "--endpoint",
        "http://example:9000",
        "status",
    ]);
    assert_eq!(cli.global.output, OutputFormat::Json);
    assert_eq!(cli.global.endpoint, "http://example:9000");
}

#[test]
fn global_flags_also_parse_after_the_subcommand() {
    // `global = true` on GlobalArgs' fields (spec §17.3's flags apply
    // uniformly): scripts and agents should be able to append `--output`
    // regardless of where in the command line it lands.
    let cli = Cli::parse_from(["mtcctl", "status", "--output", "yaml"]);
    assert_eq!(cli.global.output, OutputFormat::Yaml);
}

#[test]
fn yes_and_confirm_parse_independently() {
    let cli = Cli::parse_from(["mtcctl", "--yes", "cert", "revoke"]);
    assert!(cli.global.yes);
    assert!(!cli.global.confirm);
    assert!(cli.global.confirmed());

    let cli = Cli::parse_from(["mtcctl", "--confirm", "cert", "revoke"]);
    assert!(!cli.global.yes);
    assert!(cli.global.confirm);
    assert!(cli.global.confirmed());
}

#[test]
fn rejects_unknown_output_format() {
    let result = Cli::try_parse_from(["mtcctl", "--output", "xml", "status"]);
    assert!(result.is_err());
}

#[test]
fn rejects_unknown_subcommand() {
    let result = Cli::try_parse_from(["mtcctl", "no-such-command"]);
    assert!(result.is_err());
}

/// One parse-from-iter check per leaf declared in spec §17.3's tree
/// (`docs/mtc-architecture-spec.md` §17.3) -- every path in that diagram
/// must parse successfully, whether or not it is wired to a real operation
/// yet (ticket mtc-no9 AC: "the complete §17.3 subcommand tree declared").
#[test]
fn every_declared_leaf_parses() {
    let paths: &[&[&str]] = &[
        &["status"],
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

    for path in paths {
        let mut full = vec!["mtcctl"];
        full.extend_from_slice(path);
        let result = Cli::try_parse_from(&full);
        assert!(result.is_ok(), "{path:?} failed to parse: {result:?}");
    }
}
