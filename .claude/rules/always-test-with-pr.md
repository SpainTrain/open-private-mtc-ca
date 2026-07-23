# always-test-with-pr

> Spec: §22.13 (Required CI checks); see also §25.3 (PR best practices).

## Rule

Every PR includes tests. Never write or land code without tests. Tests ship in the
same PR as the code they cover — never in a follow-up.

## Rationale

The architecture treats CI-validated tests as the primary correctness gate for
agent-generated code (§22.13 requires `cargo test --all-features` and
`cargo test --doc` on every PR). §25.3 is explicit: "Tests in the same PR as the
code. Always." A PR is a single reviewable and revertable unit; code without its
tests is neither reviewable (the reviewer cannot see the behavior contract) nor
safely revertable (reverting the later test-PR leaves untested code behind).
Deferred tests are the first step toward the silent error-swallowing and eroded
invariants this repo is designed to prevent (§23).

## Compliant example

A PR adding `Storage::allocate_indices` that contains, in the same diff:

```text
crates/storage/src/lib.rs        # the new trait method + implementation
crates/storage/src/tests.rs      # unit tests: success path, lost lease, retry
crates/storage/tests/localstack.rs  # integration test against LocalStack DDB
```

## Non-compliant example

```text
PR #41: "Implement allocate_indices (tests to follow in #42)"
```

A PR whose description promises tests later, or whose only validation is a
manual run, does not meet the bar — regardless of how small the change is.

## Enforcement

- **CI gate**: `cargo test --all-features` and `cargo test --doc` run on every
  PR (§22.13); a PR that adds behavior without exercising it is expected to be
  caught by coverage-sensitive review.
- **Review**: reviewers reject PRs that add or change behavior without
  corresponding tests in the same diff.
