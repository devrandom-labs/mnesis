//! The `SignedRegister` aggregate: one ed25519-signed, blake3 hash-chained
//! event stream per register.
//!
//! Every event is signed by the register owner's key over a deterministic
//! preimage, and carries the digest of the previous event so the stream is
//! tamper-evident. The aggregate id is *content-addressed*: `id =
//! blake3(owner_pubkey)`, so a register's identity is bound to the key that
//! controls it (a KERI-shaped pattern — not KERI: no KEL, SAID, rotation, or
//! witnesses).
//!
//! Verification lives on the **write** side ([`Handle`]) — a command that
//! decides an event must prove the signer is authorised. The **fold**
//! ([`AggregateState::apply`]) is pure and trusts the already-accepted log. The
//! untrusted **read** side re-verifies from scratch — see
//! [`crate::projection`].

use std::collections::HashMap;
use std::fmt;

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use mnesis::{AggregateState, DomainEvent, Events, Handle, events};
use serde::{Deserialize, Serialize};

/// Domain-separation tag for the inception preimage.
const INCEPT_TAG: &[u8] = b"incept";
/// Domain-separation tag for the set preimage.
const SET_TAG: &[u8] = b"set";

/// The signed-event protocol version, independent of stream and envelope schema.
/// Only version 2 is accepted. Missing, legacy and future discriminators do not
/// fall back to a different signing preimage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum SignatureVersion {
    /// Domain-separated signing with independently hashed variable fields.
    V2,
}

impl SignatureVersion {
    /// The serialized discriminator, also bound into signing and chain digests.
    #[must_use]
    pub const fn get(self) -> u8 {
        match self {
            Self::V2 => 2,
        }
    }
}

/// An unsupported signed-event protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unsupported signature version {actual}; only version 2 is accepted")]
pub struct SignatureVersionError {
    /// The discriminator read from input.
    pub actual: u8,
}

impl TryFrom<u8> for SignatureVersion {
    type Error = SignatureVersionError;
    fn try_from(actual: u8) -> Result<Self, Self::Error> {
        if actual == Self::V2.get() {
            Ok(Self::V2)
        } else {
            Err(SignatureVersionError { actual })
        }
    }
}

impl From<SignatureVersion> for u8 {
    fn from(version: SignatureVersion) -> Self {
        version.get()
    }
}

fn signing_hasher(version: SignatureVersion, event_tag: &[u8]) -> blake3::Hasher {
    let mut hasher = blake3::Hasher::new_derive_key(
        "devrandom-labs/mnesis/examples/signed-events/signing-preimage",
    );
    hasher.update(&[version.get()]);
    hasher.update(event_tag);
    hasher
}

// ═══════════════════════════════════════════════════════════════════════════
// RegisterId — content-addressed identity: id = blake3(owner_pubkey)
// ═══════════════════════════════════════════════════════════════════════════

/// The register's identity, the blake3 digest of its owner's public key.
///
/// A 32-byte content address computable from the public key. Signing requires
/// possession of the corresponding private key. Implements [`mnesis::Id`] via the blanket impl (it
/// already carries every supertrait: `Clone + Send + Sync + Debug + Hash + Eq +
/// Display + AsRef<[u8]> + 'static`). `Display` is lowercase hex; `AsRef<[u8]>`
/// is the raw digest — the stable byte key the store uses.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct RegisterId([u8; 32]);

impl RegisterId {
    /// The content address of `pubkey`: `blake3(pubkey)`.
    #[must_use]
    pub fn from_pubkey(pubkey: &[u8; 32]) -> Self {
        Self(*blake3::hash(pubkey).as_bytes())
    }

    /// The raw 32-byte digest.
    #[must_use]
    pub const fn as_digest(&self) -> &[u8; 32] {
        &self.0
    }

    /// Rebuild the id from raw key bytes — the `$all` [`StreamKey`] a read-side
    /// consumer routes on (#333).
    ///
    /// [`StreamKey`]: mnesis_store::StreamKey
    #[must_use]
    pub fn from_key_bytes(bytes: &[u8]) -> Option<Self> {
        <[u8; 32]>::try_from(bytes).ok().map(Self)
    }
}

impl fmt::Display for RegisterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl AsRef<[u8]> for RegisterId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Events — signed, hash-chained
// ═══════════════════════════════════════════════════════════════════════════

/// The register's domain events. Both variants carry an ed25519 signature over
/// a deterministic preimage; `Set` also carries the prior event's digest (the
/// chain link).
///
/// The signature lives **inside the payload** rather than in envelope metadata:
/// this example keeps signed events self-contained. The typed facade also
/// supports envelope metadata through its metadata provider. See `README.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, DomainEvent)]
pub enum RegisterEvent {
    /// Genesis event. `prior = None`; establishes the owner. Signed over
    /// the version-2 inception preimage.
    Inception {
        /// Required signing protocol discriminator; legacy payloads are rejected.
        signature_version: SignatureVersion,
        /// The owner's ed25519 verifying key. `blake3` of this is the register id.
        owner_pubkey: [u8; 32],
        /// ed25519 signature over the inception preimage.
        #[serde(with = "sig_bytes")]
        sig: [u8; 64],
    },
    /// A key→value assignment, chained to the prior event. Signed over
    /// the version-2 set preimage.
    Set {
        /// Required signing protocol discriminator; legacy payloads are rejected.
        signature_version: SignatureVersion,
        /// The entry key.
        key: String,
        /// The entry value.
        val: String,
        /// Digest of the event immediately before this one — the chain link.
        prior_digest: [u8; 32],
        /// ed25519 signature over the set preimage.
        #[serde(with = "sig_bytes")]
        sig: [u8; 64],
    },
}

/// The version-2 inception digest, binding the protocol, event kind and owner key.
#[must_use]
pub fn inception_preimage(version: SignatureVersion, owner_pubkey: &[u8; 32]) -> [u8; 32] {
    let mut hasher = signing_hasher(version, INCEPT_TAG);
    hasher.update(owner_pubkey);
    *hasher.finalize().as_bytes()
}

/// The version-2 set digest, binding the protocol, event kind, key, value and
/// prior digest.
///
/// Key/value UTF-8 bytes are hashed independently into fixed-width
/// fields, so embedded NUL bytes cannot move a boundary between them. This
/// removes the legacy framing ambiguity; digest collision resistance remains
/// the cryptographic hash's responsibility.
#[must_use]
pub fn set_preimage(
    version: SignatureVersion,
    key: &str,
    val: &str,
    prior_digest: &[u8; 32],
) -> [u8; 32] {
    let mut hasher = signing_hasher(version, SET_TAG);
    hasher.update(blake3::hash(key.as_bytes()).as_bytes());
    hasher.update(blake3::hash(val.as_bytes()).as_bytes());
    hasher.update(prior_digest);
    *hasher.finalize().as_bytes()
}

/// The event's chain digest — the value the *next* event carries as its
/// `prior_digest`.
///
/// A deterministic, infallible function of the event's fields (each
/// variable-length field folded through its own `blake3` digest so the encoding
/// is unambiguous, with no length-prefix arithmetic). Computed identically on
/// the write side (this crate's [`Handle`]) and the untrusted read side
/// ([`crate::projection`]), so the chain is reproducible from either.
///
/// It is a structured hash of the fields rather than the raw JSON payload
/// bytes: [`AggregateState::apply`] must be infallible, and re-serialising an
/// event to JSON inside the fold is fallible — this keeps the fold panic-free
/// without an `unwrap` or a silent-sentinel digest. See `README.md`.
#[must_use]
pub fn event_digest(event: &RegisterEvent) -> [u8; 32] {
    let signature_version = match event {
        RegisterEvent::Inception {
            signature_version, ..
        }
        | RegisterEvent::Set {
            signature_version, ..
        } => signature_version,
    };
    let mut hasher =
        blake3::Hasher::new_derive_key("devrandom-labs/mnesis/examples/signed-events/event-digest");
    hasher.update(&[signature_version.get()]);
    match event {
        RegisterEvent::Inception {
            owner_pubkey, sig, ..
        } => {
            hasher.update(b"Inception\0");
            hasher.update(owner_pubkey);
            hasher.update(sig);
        }
        RegisterEvent::Set {
            key,
            val,
            prior_digest,
            sig,
            ..
        } => {
            hasher.update(b"Set\0");
            hasher.update(blake3::hash(key.as_bytes()).as_bytes());
            hasher.update(blake3::hash(val.as_bytes()).as_bytes());
            hasher.update(prior_digest);
            hasher.update(sig);
        }
    }
    *hasher.finalize().as_bytes()
}

// ═══════════════════════════════════════════════════════════════════════════
// State — the pure fold target
// ═══════════════════════════════════════════════════════════════════════════

/// The register's folded state.
#[derive(Debug, Clone, Default)]
pub struct RegisterState {
    /// The current key→value entries.
    pub entries: HashMap<String, String>,
    /// The chain head — digest of the last folded event. `None` before inception.
    pub last_digest: Option<[u8; 32]>,
    /// The owner's verifying key, seeded by `Inception`. `None` at `initial()`.
    pub owner: Option<VerifyingKey>,
}

impl AggregateState for RegisterState {
    type Event = RegisterEvent;

    fn initial() -> Self {
        Self::default()
    }

    fn apply(mut self, event: &RegisterEvent) -> Self {
        match event {
            RegisterEvent::Inception { owner_pubkey, .. } => {
                // The key was validated when the event was decided (Handle) and
                // is re-validated on the untrusted read path (projection), so a
                // persisted Inception always carries a valid key — `.ok()` here
                // only guards the unreachable corrupt-log case, never a
                // legitimate one.
                self.owner = VerifyingKey::from_bytes(owner_pubkey).ok();
                self.last_digest = Some(event_digest(event));
            }
            RegisterEvent::Set { key, val, .. } => {
                self.entries.insert(key.clone(), val.clone());
                self.last_digest = Some(event_digest(event));
            }
        }
        self
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Error
// ═══════════════════════════════════════════════════════════════════════════

/// Why a command was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// The signature did not verify against the expected key.
    #[error("signature verification failed")]
    BadSignature,
    /// The event's `prior_digest` did not match the chain head.
    #[error("broken hash chain: expected prior digest {expected:02x?}, got {actual:02x?}")]
    BrokenChain {
        /// The chain head the event should have pointed at.
        expected: [u8; 32],
        /// The `prior_digest` the event actually carried.
        actual: [u8; 32],
    },
    /// The signer is not the register's owner.
    #[error("signer is not the register owner")]
    Unauthorized,
    /// The register has not been incepted yet.
    #[error("register has not been incepted")]
    NotIncepted,
    /// The register is already incepted.
    #[error("register is already incepted")]
    AlreadyIncepted,
}

// ═══════════════════════════════════════════════════════════════════════════
// Aggregate marker + commands + handlers
// ═══════════════════════════════════════════════════════════════════════════

/// The register aggregate marker.
#[mnesis::aggregate(state = RegisterState, error = RegisterError, id = RegisterId)]
pub struct SignedRegister;

/// Incept a fresh register. The caller holds the private signing key; the
/// handler signs the genesis event.
pub struct Incept {
    /// The owner's signing key — proves control of the identity.
    pub signing_key: SigningKey,
}

/// Assign `key = val`, signed and chained onto the register head.
pub struct SubmitSet {
    /// The entry key.
    pub key: String,
    /// The entry value.
    pub val: String,
    /// The signing key — must be the register owner's, or the decision is
    /// [`RegisterError::Unauthorized`].
    pub signing_key: SigningKey,
}

impl Handle<Incept> for SignedRegister {
    fn handle(
        state: &RegisterState,
        cmd: Incept,
    ) -> Result<Option<Events<RegisterEvent>>, RegisterError> {
        if state.owner.is_some() {
            return Err(RegisterError::AlreadyIncepted);
        }
        let owner_pubkey = cmd.signing_key.verifying_key().to_bytes();
        let signature_version = SignatureVersion::V2;
        let preimage = inception_preimage(signature_version, &owner_pubkey);
        let sig = cmd.signing_key.sign(&preimage);
        Ok(Some(events![RegisterEvent::Inception {
            signature_version,
            owner_pubkey,
            sig: sig.to_bytes(),
        }]))
    }
}

impl Handle<SubmitSet> for SignedRegister {
    fn handle(
        state: &RegisterState,
        cmd: SubmitSet,
    ) -> Result<Option<Events<RegisterEvent>>, RegisterError> {
        let owner = state.owner.ok_or(RegisterError::NotIncepted)?;
        let prior_digest = state.last_digest.ok_or(RegisterError::NotIncepted)?;

        let signature_version = SignatureVersion::V2;
        let preimage = set_preimage(signature_version, &cmd.key, &cmd.val, &prior_digest);
        let sig = cmd.signing_key.sign(&preimage);
        // State-dependent crypto: the signature must verify against the *stored*
        // owner. A different signing key produces a signature valid under its
        // own key but not the owner's, so this rejects a non-owner writer.
        owner
            .verify_strict(&preimage, &sig)
            .map_err(|_| RegisterError::Unauthorized)?;

        Ok(Some(events![RegisterEvent::Set {
            signature_version,
            key: cmd.key,
            val: cmd.val,
            prior_digest,
            sig: sig.to_bytes(),
        }]))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// serde helper: [u8; 64] (serde's derived array impls stop at N = 32)
// ═══════════════════════════════════════════════════════════════════════════

/// `serde` implements `Serialize`/`Deserialize` for `[T; N]` only up to
/// `N = 32`, so the 64-byte signature needs a hand-written `#[serde(with)]`
/// codec. Serialised as a fixed-length tuple of bytes — the JSON payload the
/// [`JsonCodec`](mnesis_store::JsonCodec) persists.
mod sig_bytes {
    use core::fmt;

    use serde::de::{Error, SeqAccess, Visitor};
    use serde::ser::SerializeTuple;
    use serde::{Deserializer, Serializer};

    const SIG_LEN: usize = 64;

    pub(super) fn serialize<S: Serializer>(
        sig: &[u8; SIG_LEN],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut tuple = serializer.serialize_tuple(SIG_LEN)?;
        for byte in sig {
            tuple.serialize_element(byte)?;
        }
        tuple.end()
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; SIG_LEN], D::Error> {
        struct SigVisitor;

        impl<'de> Visitor<'de> for SigVisitor {
            type Value = [u8; SIG_LEN];

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of 64 bytes")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut sig = [0u8; SIG_LEN];
                for (index, slot) in sig.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| Error::invalid_length(index, &self))?;
                }
                Ok(sig)
            }
        }

        deserializer.deserialize_tuple(SIG_LEN, SigVisitor)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code: unwrap/expect/panic document setup invariants and assertions"
)]
mod tests {
    use super::{
        Incept, RegisterError, RegisterEvent, RegisterId, RegisterState, SignedRegister, SubmitSet,
        event_digest, inception_preimage,
    };
    use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
    use mnesis::testing::AggregateFixture;
    use mnesis::{AggregateState, Version};
    use rand_core::OsRng;

    fn keypair() -> (SigningKey, RegisterId) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());
        (signing_key, id)
    }

    /// Decide a genuine inception event via the real `Handle` path.
    fn incept_event(signing_key: &SigningKey) -> RegisterEvent {
        let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());
        let root = SignedRegister::new(id);
        root.handle(Incept {
            signing_key: signing_key.clone(),
        })
        .expect("incept decides")
        .expect("incept records an event")
        .into_iter()
        .next()
        .expect("exactly one inception event")
    }

    #[test]
    fn incept_signs_a_verifiable_inception_bound_to_the_key() {
        // Handle<Incept> must emit an inception whose owner key is the command's
        // key, whose id is blake3(pubkey), and whose signature verifies over the
        // inception preimage. Verifying (not re-signing) avoids reimplementing
        // the SUT.
        let (signing_key, id) = keypair();
        let pubkey = signing_key.verifying_key().to_bytes();
        match incept_event(&signing_key) {
            RegisterEvent::Inception {
                owner_pubkey,
                sig,
                signature_version,
            } => {
                assert_eq!(owner_pubkey, pubkey, "inception binds the owner key");
                assert_eq!(
                    RegisterId::from_pubkey(&owner_pubkey),
                    id,
                    "id must be blake3(pubkey)"
                );
                let vk = VerifyingKey::from_bytes(&owner_pubkey).expect("valid key");
                vk.verify_strict(
                    &inception_preimage(signature_version, &owner_pubkey),
                    &Signature::from_bytes(&sig),
                )
                .expect("inception signature must verify");
            }
            set @ RegisterEvent::Set { .. } => panic!("expected Inception, got {set:?}"),
        }
    }

    #[test]
    fn submit_set_before_incept_is_rejected() {
        let (signing_key, id) = keypair();
        let _ = AggregateFixture::<SignedRegister>::with_id(id)
            .given([])
            .when(SubmitSet {
                key: "a".to_owned(),
                val: "1".to_owned(),
                signing_key,
            })
            .then_expect_error(RegisterError::NotIncepted);
    }

    #[test]
    fn second_incept_is_rejected() {
        let (signing_key, id) = keypair();
        let inception = incept_event(&signing_key);
        let _ = AggregateFixture::<SignedRegister>::with_id(id)
            .given([inception])
            .when(Incept { signing_key })
            .then_expect_error(RegisterError::AlreadyIncepted);
    }

    #[test]
    fn set_by_a_non_owner_is_unauthorized() {
        let (owner_key, id) = keypair();
        let inception = incept_event(&owner_key);
        let attacker = SigningKey::generate(&mut OsRng);
        let _ = AggregateFixture::<SignedRegister>::with_id(id)
            .given([inception])
            .when(SubmitSet {
                key: "a".to_owned(),
                val: "x".to_owned(),
                signing_key: attacker,
            })
            .then_expect_error(RegisterError::Unauthorized);
    }

    #[test]
    fn incept_then_sets_fold_state_and_chain() {
        // Sequence/protocol: multi-step on one root, then a fresh replay must
        // rebuild identical state, and each Set must link to the prior digest.
        let (signing_key, id) = keypair();
        let mut root = SignedRegister::new(id);

        let e1 = root
            .handle(Incept {
                signing_key: signing_key.clone(),
            })
            .expect("incept")
            .expect("event");
        let inception = e1.first().clone();
        root.commit_persisted(&e1).expect("root is usable");
        let d1 = event_digest(&inception);

        let e2 = root
            .handle(SubmitSet {
                key: "a".to_owned(),
                val: "1".to_owned(),
                signing_key: signing_key.clone(),
            })
            .expect("set a")
            .expect("event");
        let set1 = e2.first().clone();
        assert!(
            matches!(&set1, RegisterEvent::Set { prior_digest, .. } if *prior_digest == d1),
            "set1 must chain to the inception digest"
        );
        root.commit_persisted(&e2).expect("root is usable");
        let d2 = event_digest(&set1);

        let e3 = root
            .handle(SubmitSet {
                key: "b".to_owned(),
                val: "2".to_owned(),
                signing_key,
            })
            .expect("set b")
            .expect("event");
        let set2 = e3.first().clone();
        assert!(
            matches!(&set2, RegisterEvent::Set { prior_digest, .. } if *prior_digest == d2),
            "set2 must chain to set1's digest"
        );
        root.commit_persisted(&e3).expect("root is usable");

        assert_eq!(
            root.state().unwrap().entries.get("a"),
            Some(&"1".to_owned())
        );
        assert_eq!(
            root.state().unwrap().entries.get("b"),
            Some(&"2".to_owned())
        );
        assert_eq!(root.state().unwrap().last_digest, Some(event_digest(&set2)));

        let mut replayed = SignedRegister::new(id);
        replayed.replay(Version::INITIAL, &inception).expect("v1");
        replayed
            .replay(Version::new(2).unwrap(), &set1)
            .expect("v2");
        replayed
            .replay(Version::new(3).unwrap(), &set2)
            .expect("v3");
        assert_eq!(
            replayed.state().unwrap().entries,
            root.state().unwrap().entries
        );
        assert_eq!(
            replayed.state().unwrap().last_digest,
            root.state().unwrap().last_digest
        );
    }

    #[test]
    fn apply_is_a_pure_fold_over_the_signed_event() {
        // apply folds a decided event with no verification — proves the fold
        // does not depend on crypto (that is Handle's / the projector's job).
        let (signing_key, _id) = keypair();
        let inception = incept_event(&signing_key);
        let state = RegisterState::initial().apply(&inception);
        assert!(state.owner.is_some());
        assert_eq!(state.last_digest, Some(event_digest(&inception)));
    }
}
