//! Vector discovery and file loading.
//!
//! [`discover_vector_files`] walks a directory tree collecting `*.json`
//! files in a deterministic (sorted) order — "the runner discovers every
//! vector" (spec §19.4 AC) rather than requiring a hand-maintained manifest,
//! so adding a vector is "drop a JSON file in the right subdirectory" (see
//! `conformance/vectors/README.md`). [`load_vector_file`] parses one file
//! against the [`crate::schema::Vector`] schema.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::schema::Vector;

/// A vector file could not be loaded.
#[derive(Debug, Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("failed to read vector file {path}: {source}")]
    Io {
        /// The file that failed to read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file's contents did not match the vector schema.
    #[error("failed to parse vector file {path} as a conformance vector: {source}")]
    Schema {
        /// The file that failed to parse.
        path: PathBuf,
        /// The underlying JSON/schema error.
        #[source]
        source: serde_json::Error,
    },
}

/// Recursively collects every `*.json` file under `root`, in sorted order
/// (deterministic discovery, and a stable iteration order for reproducible
/// pass/fail output).
///
/// Non-JSON files (e.g. `README.md`) and dotfiles are skipped, so the vectors
/// directory can carry its own documentation alongside the fixtures.
///
/// # Errors
///
/// Propagates any [`std::io::Error`] from reading a directory entry.
pub fn discover_vector_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect(root, &mut out)?;
    out.sort();
    Ok(out)
}

/// Recursion helper for [`discover_vector_files`].
fn collect(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect(&path, out)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
    Ok(())
}

/// Loads and parses one vector file.
///
/// # Errors
///
/// [`LoadError::Io`] if the file cannot be read; [`LoadError::Schema`] if its
/// contents do not match the [`Vector`] schema (missing required field,
/// unknown `kind`, wrong field type, …).
pub fn load_vector_file(path: &Path) -> Result<Vector, LoadError> {
    let contents = fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| LoadError::Schema {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{discover_vector_files, load_vector_file, LoadError};

    /// A scratch directory under `target/` for this test binary only, so
    /// parallel test runs (and repeated local runs) never collide. Not the
    /// real `conformance/vectors/` tree — that is exercised by
    /// `tests/conformance.rs`.
    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mtc-conformance-loader-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn discovers_json_files_recursively_and_sorted() {
        let dir = scratch_dir("discover");
        fs::create_dir_all(dir.join("checkpoint")).unwrap();
        fs::create_dir_all(dir.join("inclusion-proof")).unwrap();
        fs::write(dir.join("checkpoint/b.json"), "{}").unwrap();
        fs::write(dir.join("checkpoint/a.json"), "{}").unwrap();
        fs::write(dir.join("inclusion-proof/c.json"), "{}").unwrap();
        fs::write(dir.join("README.md"), "not a vector").unwrap();
        fs::write(dir.join(".hidden.json.bak"), "not json").unwrap();

        let found = discover_vector_files(&dir).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "checkpoint/a.json",
                "checkpoint/b.json",
                "inclusion-proof/c.json",
            ],
        );
    }

    #[test]
    fn discover_on_missing_directory_returns_empty() {
        let dir = scratch_dir("missing").join("does-not-exist");
        assert_eq!(
            discover_vector_files(&dir).unwrap(),
            Vec::<std::path::PathBuf>::new()
        );
    }

    #[test]
    fn loads_a_well_formed_vector() {
        let dir = scratch_dir("load-ok");
        let path = dir.join("v.json");
        fs::write(
            &path,
            r#"{
                "kind": "log_entry",
                "id": "example",
                "description": "d",
                "wire_hex": "0000",
                "parse": { "outcome": "accept", "fields": { "variant": "null" } }
            }"#,
        )
        .unwrap();
        let vector = load_vector_file(&path).unwrap();
        assert_eq!(vector.id(), "example");
    }

    #[test]
    fn reports_schema_errors_with_the_path() {
        let dir = scratch_dir("load-bad-schema");
        let path = dir.join("v.json");
        // Missing the required "wire_hex" field.
        fs::write(
            &path,
            r#"{"kind": "log_entry", "id": "x", "description": "d", "parse": {"outcome": "accept"}}"#,
        )
        .unwrap();
        match load_vector_file(&path) {
            Err(LoadError::Schema { path: p, .. }) => assert_eq!(p, path),
            other => panic!("expected LoadError::Schema, got {other:?}"),
        }
    }

    #[test]
    fn reports_io_errors_for_a_missing_file() {
        let dir = scratch_dir("load-missing");
        let path = dir.join("does-not-exist.json");
        match load_vector_file(&path) {
            Err(LoadError::Io { path: p, .. }) => assert_eq!(p, path),
            other => panic!("expected LoadError::Io, got {other:?}"),
        }
    }
}
