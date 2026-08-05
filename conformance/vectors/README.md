# Conformance test vectors

Spec §19.4 ("Spec conformance suite"): clean-room test vectors, an ongoing CI
gate. This directory holds the vectors; `crates/conformance/` is the runner
that discovers, loads, and evaluates every one of them against the real `mtc`
crate types. See `../README.md` for how the two fit together and how this
rides the repository's existing required CI check.

## Layout

```
conformance/vectors/
  README.md              <- this file
  checkpoint/*.json       <- mtc::Checkpoint vectors
  inclusion-proof/*.json  <- mtc::InclusionProof vectors
  log-entry/*.json        <- mtc::LogEntry vectors
```

One vector is one JSON file. The runner discovers every `*.json` file under
this directory recursively (`mtc_conformance::discover_vector_files`), so the
subdirectory layout above is a convention for humans, not something the
runner hard-codes — a new `<kind>/` subdirectory needs no runner change, only
a matching `Vector` variant in `crates/conformance/src/schema.rs` (see
"Adding a new *kind*" below).

## The vector format

Every vector file has this shape:

```jsonc
{
  "kind": "checkpoint",          // "checkpoint" | "inclusion_proof" | "log_entry"
  "id": "checkpoint-accept-001", // unique; matches the filename stem by convention
  "description": "...",          // what this vector exercises and why
  "wire_hex": "0a70726f64...",   // the input bytes, lowercase hex, no "0x"/separators
  "parse": {
    "outcome": "accept",         // "accept" | "reject"
    "fields": { ... }            // required iff outcome == "accept"; kind-specific
    // "error_class": "..."      // required iff outcome == "reject"
  },
  "verify": {                    // optional; only meaningful when parse.outcome == "accept"
    "outcome": "accept",         // "accept" | "reject"
    ... verify material ...,     // kind-specific (a key, or a leaf hash + root)
    "error_class": "..."         // required iff outcome == "reject"
  }
}
```

The runner (`mtc_conformance::evaluate`) does, per vector:

1. Decode `wire_hex`.
2. Parse it through the real type's real parser (`Checkpoint::parse_tls_presentation`,
   `InclusionProof::tls_parse_exact`, `LogEntry::tls_parse_exact` — never a
   reimplementation).
3. Check the parse outcome against `parse.outcome`:
   - `accept`: the parsed value's fields must equal `parse.fields`, field by
     field. A mismatch is reported as a **structural diff** — one line per
     differing field, `expected` vs. `actual` — not a bare assertion failure.
   - `reject`: the actual error's `error_class` must match (see below).
4. If `verify` is present (and `parse.outcome` was `accept`), run the real
   `verify()` and check its outcome the same way.

### `error_class` matching

`error_class` is checked as a **substring of the actual error's `{:?}` (Debug)
rendering**, not full structural equality. `"TrailingBytes"` matches a bare
`WireError::TrailingBytes { .. }` and also a `CheckpointParseError::Wire(WireError::TrailingBytes { .. })`
wrapping it — so you only need to name the specific variant that fired, not
reproduce its exact field payload (offsets and lengths are useful in the
description but are not part of the match). Pick the value straight from the
error enum's variant name: `TrustAnchorIdEmpty`, `LogIdNotUtf8`,
`TrailingBytes`, `UnexpectedEof`, `LengthOverflow`, `DepthLimitExceeded`,
`ZeroWidthElement`, `InvalidValue` (all `mtc::WireError` / `mtc::CheckpointParseError`
variants), `IndexOutOfRange`, `MalformedPath`, `RootMismatch`,
`NonMonotonicSizes`, `TreeTooSmall` (`mtc::ProofError`), `BadSignature`,
`MalformedSignature`, `MalformedKey`, `AlgorithmMismatch` (`mtc::CheckpointVerifyError`
/ `mtc::VerifyError`).

### Per-kind `fields` / verify material

**`checkpoint`** (`mtc::Checkpoint<Signed>`):

```jsonc
"fields": {
  "log_id": "prod-log-1",
  "tree_size": 12345,
  "root_hash_hex": "7f7f...7f",   // 32 bytes
  "signed_at": 1700000000,
  "signature_hex": "5cef...a5"    // 64 bytes, ECDSA P-256 r||s (IEEE P1363)
}
```

```jsonc
"verify": {
  "outcome": "accept",
  "verifying_key_spki_hex": "3059...99"  // DER SubjectPublicKeyInfo
}
```

**`inclusion_proof`** (`mtc::InclusionProof`):

```jsonc
"fields": {
  "tree_size": 13,
  "leaf_index": 5,
  "audit_path_hex": ["...", "...", ...]  // leaf-to-root, each 32 bytes
}
```

```jsonc
"verify": {
  "outcome": "accept",
  "leaf_hash_hex": "5141...8f",   // hash_leaf(entry bytes), NOT the raw entry
  "root_hash_hex": "96a5...60"
}
```

**`log_entry`** (`mtc::LogEntry`) — no `verify` step; a log entry has no
self-contained verification:

```jsonc
// LogEntry::Null
"fields": { "variant": "null" }

// LogEntry::Certificate
"fields": {
  "variant": "certificate",
  "subject_type": "tls",
  "subject_info_hash_hex": "1111...11",  // 32 bytes
  "claim_count": 1
}
```

## Adding a vector

**Preferred: extend `crates/conformance/examples/generate_vectors.rs` and
re-run it.** Ticket `test-conformance-runner`'s hard rule: vector bytes come
from the real `mtc` serializers, never hand-invented byte layouts. The
generator is the audit trail for "these bytes are what `Checkpoint::sign` /
`InclusionProof::generate` / `LogEntry::certificate` actually produce" —
add a case there (a new field value, a new mutation of existing real bytes,
a new structure) rather than writing a JSON file by hand:

```sh
cargo run -p mtc-conformance --example generate_vectors
```

This overwrites every generator-owned vector file, so review the diff before
committing — a change to the generator can shift bytes for existing vectors
too (e.g. changing the signing key or a chosen field value).

**When hand-authoring is unavoidable** (a byte prefix too short or too
malformed for any real value to produce — e.g. a bare length-prefix byte
testing a floor check before any real structure could exist): write the JSON
file directly, but the `description` field must say exactly where the bytes
come from. Two vectors in this suite do this
(`checkpoint/checkpoint-reject-empty-trust-anchor-id.json`,
`checkpoint/checkpoint-reject-non-utf8-log-id.json`) — both cite the exact
`crates/mtc` unit test whose already-asserted-correct byte pattern they reuse.
An `error_class` you cannot point at a real parser run is not a conformance
vector.

Either way:

1. Pick an `id` unique across the whole `vectors/` tree (the filename stem,
   by convention: `<kind>-<accept|reject>-<slug>.json`).
2. Run the suite (`make test-conformance` or `cargo test -p mtc-conformance`)
   and confirm your new vector's `PASS` line appears and the total count grew
   by one.
3. If it's a `reject` vector, deliberately try the *wrong* `error_class`
   first and confirm the runner reports the real one in its failure detail —
   this is the fastest way to get the exact variant name right (see the
   `seed_vectors_satisfy_the_ac_minimums` test in
   `crates/conformance/tests/conformance.rs` for the minimum shape a
   compliant vector set must keep).

### Adding a new *kind*

Only needed for a spec structure this suite does not cover yet (e.g. a future
`ConsistencyProof` or `Tile` vector kind). Add a variant to `mtc_conformance::schema::Vector`
and a matching `evaluate_*` function in `mtc_conformance::runner` — the
directory-discovery and reporting machinery in `crates/conformance/tests/conformance.rs`
needs no change; it dispatches purely on the `kind` field.

## Clean-room vectors vs. draft vectors

Every vector currently in this directory is **clean-room**: authored against
this repository's own `mtc` implementation, proving our serializer and parser
agree with *themselves* (round-trip, and the specific reject cases above).
None of them is byte-exact-checked against the
`draft-ietf-plants-merkle-tree-certs-03` text itself — that is bead
`mtc-qka.5`, tracked separately. When those draft-derived vectors land, they
drop into this same format and these same subdirectories: a draft vector is
just a `wire_hex` sourced from the draft's own published test vector (instead
of from `generate_vectors.rs`) with `fields`/`verify` filled in by hand from
the draft text, evaluated by the exact same runner. If the draft's byte
layout for a structure ever needs a field this schema does not have yet,
extend the kind's `*Fields`/`*VerifyMaterial` struct in `schema.rs` rather
than working around it — the whole point of a shared schema is that both
vector sources stay comparable.
