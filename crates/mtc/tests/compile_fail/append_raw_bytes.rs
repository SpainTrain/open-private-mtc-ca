//! Write-path leaf-framing invariant (crypto audit 2026-08-05, Finding 2;
//! ticket mtc-qka.10): `MerkleTree::append` takes a `LeafBytes` produced only by
//! `LogEntry::leaf_bytes`, never a raw `&[u8]`. Appending un-framed bytes — the
//! trap that silently fails all relying-party verification — must be a compile
//! error, not a runtime one.

fn main() {
    let mut tree: mtc::MerkleTree = mtc::MerkleTree::new();
    // Raw bytes with no `00 00` / `00 01` entry-type discriminant frame: exactly
    // the mistake the audit oracle demonstrated. This must not typecheck.
    tree.append(b"raw tbs bytes with no discriminant");
}
