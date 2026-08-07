//! End-to-end check of the compiled binary's exit behavior for
//! unimplemented leaves (ticket mtc-no9 AC: "unimplemented leaves exit
//! non-zero with a clear 'not yet implemented' message"; "Errors map to
//! distinct exit codes; stderr for diagnostics, stdout for payload").
//!
//! No server needed: every case here fails before `mtcctl` would make any
//! admin API call.

use std::process::Command;

#[test]
fn unimplemented_leaf_exits_non_zero_with_a_clear_message_on_stderr() {
    let output = Command::new(env!("CARGO_BIN_EXE_mtcctl"))
        .args(["batch", "list"])
        .output()
        .expect("mtcctl runs to completion");

    assert!(!output.status.success(), "expected a non-zero exit code");
    assert_eq!(output.status.code(), Some(3));

    // stdout carries payload only -- nothing should land there for a
    // command that never ran.
    assert!(output.stdout.is_empty(), "stdout must stay empty on error");

    let stderr = String::from_utf8(output.stderr).expect("stderr is UTF-8");
    assert!(stderr.contains("batch list"));
    assert!(stderr.contains("not yet implemented"));
}

#[test]
fn different_unimplemented_leaves_still_share_exit_code_3() {
    // `NotImplemented` is one variant regardless of which leaf triggered
    // it (Api/Connection failures, exit codes 4/5, are the ones that vary
    // by *kind* of failure -- see crates/mtcctl/src/error.rs).
    let cases: [&[&str]; 2] = [&["repl"], &["adapter", "pause"]];
    for args in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_mtcctl"))
            .args(args)
            .output()
            .expect("mtcctl runs to completion");
        assert_eq!(output.status.code(), Some(3), "args: {args:?}");
    }
}

#[test]
fn unknown_subcommand_exits_with_claps_usage_code() {
    // clap's own usage-error exit code (2), distinct from every
    // `CliError` code (see error.rs's doc comment on why codes start at 3).
    let output = Command::new(env!("CARGO_BIN_EXE_mtcctl"))
        .args(["no-such-command"])
        .output()
        .expect("mtcctl runs to completion");
    assert_eq!(output.status.code(), Some(2));
}
