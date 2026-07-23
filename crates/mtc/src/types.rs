//! Domain newtypes for spec section-2 concepts.
//!
//! Integer newtypes follow spec section 22.1: each is a distinct wrapper
//! struct (never a type alias — rule `use-newtypes`), so the compiler refuses
//! to pass an [`Epoch`] where a [`TreeSize`] is expected. String identifiers
//! follow the phantom-type pattern of spec section 22.5: [`LogId`] and
//! [`BatchId`] are distinct compile-time types with identical runtime
//! representation.

use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::str::FromStr;

use crate::error::{HashOutputError, IdError};

/// Zero-based position of one entry in the issuance log.
///
/// Models the leaf position within the append-only Merkle tree of issued
/// certificate entries (spec section 2, "Issuance log"): each issuance appends
/// one `TBSCertificateLogEntry` at the next index, and a subtree is the range
/// of consecutive indices `[start, end)` (section 2, "Subtree"). An inclusion
/// proof walks from the leaf at a given index up to a subtree root.
///
/// Derives per spec section 22.1: `Copy + Clone + PartialEq + Eq + Hash +
/// Debug`. The compiler rejects passing an [`Index`] where a [`TreeSize`] or
/// [`Epoch`] is expected.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Index(pub u64);

/// Number of entries in the issuance log at a point in time.
///
/// Models the tree-size half of a checkpoint: a checkpoint is a signed
/// `(tree size, root hash)` commitment to the issuance log (spec section 2,
/// "Checkpoint"). A tree size of `n` commits to the entries at indices
/// `[0, n)`.
///
/// Derives per spec section 22.1: `Copy + Clone + PartialEq + Eq + Hash +
/// Debug`. Distinct from [`Index`] and [`Epoch`] at compile time.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct TreeSize(pub u64);

/// Monotonic era counter under which batches are appended to the issuance log.
///
/// The issuance log (spec section 2, "Issuance log") grows in batches of
/// consecutive entries; abandoned batches leave gaps filled with the
/// spec-defined `null_entry` placeholder (section 2, "`null_entry`"). The
/// epoch is the write path's fencing token: index allocations and batch
/// commits are only valid under the current epoch, so a writer that lost its
/// lease cannot corrupt the log with stale appends (spec section 11).
///
/// Derives per spec section 22.1: `Copy + Clone + PartialEq + Eq + Hash +
/// Debug`. Distinct from [`Index`] and [`TreeSize`] at compile time — see
/// `tests/compile_fail/epoch_not_tree_size.rs`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Epoch(pub u64);

/// A 32-byte SHA-256 hash output.
///
/// Models every hash-valued quantity of spec section 2: the node hashes of
/// the issuance log's Merkle tree, the root-hash half of a checkpoint's
/// signed `(tree size, root hash)` commitment ("Checkpoint"), the sibling
/// hashes of an inclusion proof ("Inclusion proof"), and predistributed
/// landmark subtree hashes ("Landmark").
///
/// `Debug` renders the bytes as lowercase hex for readable logs and test
/// failures.
///
/// # Examples
///
/// ```
/// use mtc::HashOutput;
///
/// let root = HashOutput([0u8; 32]);
/// assert_eq!(root.as_bytes().len(), HashOutput::LEN);
///
/// // Fallible conversion from an untrusted slice returns a typed error
/// // instead of panicking (spec section 22.6).
/// let err = HashOutput::try_from(&[0u8; 31][..]).unwrap_err();
/// assert_eq!(
///     err,
///     mtc::HashOutputError::InvalidLength { expected: 32, actual: 31 },
/// );
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct HashOutput(pub [u8; 32]);

impl HashOutput {
    /// The length of a hash output in bytes (SHA-256).
    pub const LEN: usize = 32;

    /// Borrows the raw hash bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Consumes the newtype and returns the raw hash bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for HashOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HashOutput(")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(")")
    }
}

impl From<[u8; 32]> for HashOutput {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl TryFrom<&[u8]> for HashOutput {
    type Error = HashOutputError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| HashOutputError::InvalidLength {
                expected: Self::LEN,
                actual: bytes.len(),
            })?;
        Ok(Self(array))
    }
}

/// A phantom-typed string identifier (spec section 22.5).
///
/// `Id<T>` disambiguates the string-valued names of spec section-2 artifacts
/// at compile time: [`LogId`] (`Id<LogTag>`) names an issuance log and
/// [`BatchId`] (`Id<BatchTag>`) names a batch of consecutive entries. The tag
/// type parameter exists only at compile time — every `Id<T>` is a plain
/// `String` at runtime, so the disambiguation is zero-cost.
///
/// Identifiers are `Clone` but deliberately **not** `Copy` (spec
/// section 22.1): they own heap-allocated string data.
///
/// Construction is fallible: an empty identifier cannot name anything and is
/// rejected with [`IdError::Empty`].
///
/// # Examples
///
/// ```
/// use mtc::{BatchId, LogId};
///
/// let log: LogId = LogId::new("prod-log-1")?;
/// let batch: BatchId = BatchId::new("batch-42")?;
/// assert_eq!(log.as_str(), "prod-log-1");
/// assert_eq!(batch.to_string(), "batch-42");
/// // `log == batch` would not compile: distinct types (spec section 22.5).
/// # Ok::<(), mtc::IdError>(())
/// ```
pub struct Id<T: ?Sized> {
    value: String,
    _phantom: PhantomData<T>,
}

impl<T: ?Sized> Id<T> {
    /// Creates an identifier from a non-empty string.
    ///
    /// # Errors
    ///
    /// Returns [`IdError::Empty`] if `value` is the empty string.
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        Ok(Self {
            value,
            _phantom: PhantomData,
        })
    }

    /// Borrows the identifier as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Consumes the identifier and returns the underlying `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.value
    }
}

// Manual trait impls instead of derives: a derive would add a spurious
// `T: Clone` (etc.) bound on the phantom tag type, which is never a value.

impl<T: ?Sized> Clone for Id<T> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T: ?Sized> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: ?Sized> Eq for Id<T> {}

impl<T: ?Sized> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T: ?Sized> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Id").field(&self.value).finish()
    }
}

impl<T: ?Sized> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value)
    }
}

impl<T: ?Sized> AsRef<str> for Id<T> {
    fn as_ref(&self) -> &str {
        &self.value
    }
}

impl<T: ?Sized> FromStr for Id<T> {
    type Err = IdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl<T: ?Sized> TryFrom<String> for Id<T> {
    type Error = IdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Phantom tag for issuance-log identifiers (spec section 2, "Issuance log").
///
/// Never instantiated; exists only as the type parameter of [`LogId`].
pub struct LogTag;

/// Phantom tag for batch identifiers.
///
/// A batch is a run of consecutive issuance-log entries — a subtree
/// `[start, end)` in spec section-2 terms — and abandoned batches leave
/// `null_entry` gaps (spec section 2, "Subtree" and "`null_entry`"). Never
/// instantiated; exists only as the type parameter of [`BatchId`].
pub struct BatchTag;

/// Identifier of an issuance log (spec section 2, "Issuance log").
///
/// A distinct compile-time type from [`BatchId`] via the phantom-type pattern
/// (spec section 22.5); identical to it at runtime. See
/// `tests/compile_fail/log_id_not_batch_id.rs` for the compile-time proof.
pub type LogId = Id<LogTag>;

/// Identifier of a batch of consecutive issuance-log entries (spec section 2,
/// "Subtree"; abandoned batches yield "`null_entry`" gaps).
///
/// A distinct compile-time type from [`LogId`] via the phantom-type pattern
/// (spec section 22.5); identical to it at runtime.
pub type BatchId = Id<BatchTag>;

#[cfg(test)]
mod tests {
    use core::mem::{align_of, size_of};

    use super::{BatchId, Epoch, HashOutput, Index, LogId, TreeSize};
    use crate::error::{HashOutputError, IdError};

    #[test]
    fn integer_newtypes_are_copy_and_comparable() {
        let index = Index(7);
        let copied = index; // Copy: original stays usable.
        assert_eq!(index, copied);
        assert_ne!(TreeSize(1), TreeSize(2));
        assert_eq!(Epoch(3), Epoch(3));
    }

    #[test]
    fn integer_newtypes_expose_their_value() {
        assert_eq!(Index(7).0, 7);
        assert_eq!(TreeSize(256).0, 256);
        assert_eq!(Epoch(0).0, 0);
    }

    #[test]
    fn integer_newtypes_debug_format_names_the_type() {
        assert_eq!(format!("{:?}", Index(1)), "Index(1)");
        assert_eq!(format!("{:?}", TreeSize(2)), "TreeSize(2)");
        assert_eq!(format!("{:?}", Epoch(3)), "Epoch(3)");
    }

    #[test]
    fn hash_output_roundtrips_from_slice() {
        let bytes = [0xabu8; 32];
        let hash = HashOutput::try_from(&bytes[..]).unwrap();
        assert_eq!(hash, HashOutput(bytes));
        assert_eq!(hash.as_bytes(), &bytes);
        assert_eq!(hash.to_bytes(), bytes);
    }

    #[test]
    fn hash_output_rejects_wrong_lengths() {
        for len in [0usize, 31, 33, 64] {
            let bytes = vec![0u8; len];
            let err = HashOutput::try_from(&bytes[..]).unwrap_err();
            assert_eq!(
                err,
                HashOutputError::InvalidLength {
                    expected: HashOutput::LEN,
                    actual: len,
                },
            );
        }
    }

    #[test]
    fn hash_output_debug_is_hex() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0x01;
        bytes[31] = 0xff;
        let rendered = format!("{:?}", HashOutput(bytes));
        assert_eq!(
            rendered,
            "HashOutput(01000000000000000000000000000000000000000000000000000000000000ff)",
        );
    }

    #[test]
    fn ids_construct_clone_and_display() {
        let log = LogId::new("prod-log-1").unwrap();
        let cloned = log.clone(); // Clone but not Copy: String-backed.
        assert_eq!(log, cloned);
        assert_eq!(log.as_str(), "prod-log-1");
        assert_eq!(log.to_string(), "prod-log-1");
        assert_eq!(log.as_ref(), "prod-log-1");
        assert_eq!(cloned.into_string(), "prod-log-1");
    }

    #[test]
    fn ids_reject_empty_strings() {
        assert_eq!(LogId::new("").unwrap_err(), IdError::Empty);
        assert_eq!("".parse::<BatchId>().unwrap_err(), IdError::Empty);
        assert_eq!(
            BatchId::try_from(String::new()).unwrap_err(),
            IdError::Empty
        );
    }

    #[test]
    fn ids_parse_from_str_and_string() {
        let parsed: BatchId = "batch-42".parse().unwrap();
        assert_eq!(parsed.as_str(), "batch-42");
        let converted = LogId::try_from(String::from("log-a")).unwrap();
        assert_eq!(converted.as_str(), "log-a");
    }

    #[test]
    fn ids_debug_format_hides_the_phantom() {
        let log = LogId::new("log-a").unwrap();
        assert_eq!(format!("{log:?}"), "Id(\"log-a\")");
    }

    #[test]
    fn phantom_typed_ids_are_identical_at_runtime() {
        // Spec section 22.5: different types at compile time, identical at
        // runtime — the phantom tag adds no size or alignment.
        assert_eq!(size_of::<LogId>(), size_of::<String>());
        assert_eq!(size_of::<BatchId>(), size_of::<String>());
        assert_eq!(align_of::<LogId>(), align_of::<BatchId>());
    }

    #[test]
    fn phantom_typed_ids_are_distinct_compile_time_types() {
        use core::any::TypeId;
        assert_ne!(TypeId::of::<LogId>(), TypeId::of::<BatchId>());
    }
}
