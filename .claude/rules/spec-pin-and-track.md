# spec-pin-and-track

> Spec: §28 (References — spec tracking workflow); §23.2.

## Rule

Implementation work pins to a specific published MTC draft revision
(`draft-ietf-plants-merkle-tree-certs`, `-03` at spec v0.6). The WG GitHub
`main` (`github.com/ietf-plants-wg/merkle-tree-certs`) is watched for in-flight
changes; non-trivial divergences from our pinned revision file Beads tickets.

## Rationale

§28 (spec tracking workflow): pin to a specific published draft revision for
implementation; when WG GitHub `main` diverges from the latest published draft,
that divergence is the next revision — review it for changes that will affect
our implementation. Building against a moving target makes wire-format and
proof-format code unverifiable: two PRs written a week apart could implement
two different drafts. Pinning gives every encoder, decoder, and test vector a
single ground truth; tracking ensures upcoming changes arrive as scheduled,
ticketed work (via Beads) instead of surprise breakage at the next draft bump.

## Compliant example

```text
- Code and fixtures cite the pinned revision:
  // Encodes Assertion per draft-ietf-plants-merkle-tree-certs-03 §4.2
- WG main adds a field to CheckpointData ahead of -04
  -> Beads ticket filed: "SPEC-TRACK: CheckpointData gains extensions field
     in WG main; assess impact on serializer + fixtures before -04"
- The `update-spec-version.md` skill (§23.1) drives the eventual re-pin.
```

## Non-compliant example

```text
PR: "Match latest spec from WG main"
- Serializer changed to follow an unmerged commit on WG GitHub main
- Pinned revision unchanged everywhere else; fixtures now disagree with the
  cited draft; no Beads ticket recording the divergence
```

## Enforcement

- **CI gate**: a CI job pulls the WG GitHub `main` daily and diffs against our
  pinned version; significant changes file a Beads ticket automatically (§28).
- **Review**: changes to wire/proof formats must cite the pinned draft
  revision; implementing unpublished spec changes without a re-pin decision is
  rejected.
