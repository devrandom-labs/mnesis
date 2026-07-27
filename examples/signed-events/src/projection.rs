//! Untrusted read-side re-verification.
//!
//! A consumer reading events back off the store does **not** trust the stored
//! bytes: [`RegisterProjector`] re-checks every signature and every chain link
//! before folding an event into the [`RegisterView`]. A forged, tampered, or
//! misfiled event is rejected with an `Err`, never folded — the exact opposite
//! of the aggregate's write-side fold ([`AggregateState::apply`]), which trusts
//! the already-accepted log.
//!
//! # Stream attribution (#333/#345)
//!
//! A `Set` event carries no register id in its payload — the stream *is* the
//! identity — so on an `$all` read (events from many registers interleaved)
//! the fold attributes each event to its register through the origin
//! `StreamKey` the store stamps beside every `$all` item (#333).
//! [`RegisterProjector`] overrides
//! [`Projector::apply_attributed`](mnesis_store::Projector::apply_attributed)
//! and decodes the [`RegisterId`] from that key; the keyless
//! [`Projector::apply`](mnesis_store::Projector::apply) is the error path
//! ([`ViewError::Unattributed`]). No driver-side routing shim. See
//! `README.md`.

use std::collections::HashMap;

use ed25519_dalek::{Signature, VerifyingKey};
use mnesis_store::{Projector, StreamKey};

use crate::domain::{RegisterEvent, RegisterId, event_digest, inception_preimage, set_preimage};

/// Per-register verification context tracked as events are folded.
#[derive(Debug, Clone, Copy)]
struct ChainHead {
    owner: VerifyingKey,
    last_digest: [u8; 32],
}

/// The read model: verified entries per register, plus the per-register
/// verification state the projector needs to check the next event.
#[derive(Debug, Default)]
pub struct RegisterView {
    /// The projection result: verified key→value entries per register.
    pub registers: HashMap<RegisterId, HashMap<String, String>>,
    /// Per-register `(owner, chain head)` — verification state.
    chains: HashMap<RegisterId, ChainHead>,
}

impl RegisterView {
    /// The verified entries of `id`, if the register has been folded.
    #[must_use]
    pub fn entries_of(&self, id: &RegisterId) -> Option<&HashMap<String, String>> {
        self.registers.get(id)
    }
}

/// Why the read side rejected an event.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// The signature did not verify against the register owner's key.
    #[error("signature verification failed")]
    BadSignature,
    /// The event's `prior_digest` did not match the tracked chain head.
    #[error("broken hash chain: expected {expected:02x?}, got {actual:02x?}")]
    BrokenChain {
        /// The chain head the event should have pointed at.
        expected: [u8; 32],
        /// The `prior_digest` the event actually carried.
        actual: [u8; 32],
    },
    /// The event was stored under a stream id that is not `blake3(owner_pubkey)`.
    #[error("inception id is not blake3(owner_pubkey)")]
    IdMismatch,
    /// A `Set` arrived for a register with no prior `Inception`.
    #[error("register not incepted")]
    NotIncepted,
    /// A second `Inception` arrived for an already-incepted register.
    #[error("register already incepted")]
    AlreadyIncepted,
    /// The fold was invoked without stream attribution (keyless `apply`, or a
    /// per-stream item) — a multi-stream re-verifying fold requires the origin
    /// `StreamKey`.
    #[error("no stream attribution: the $all StreamKey is required")]
    Unattributed,
    /// The `$all` stream key was not a valid 32-byte register id.
    #[error("stream key is not a 32-byte register id")]
    BadStreamKey,
}

/// The read-side re-verifying fold.
#[derive(Debug, Default, Clone, Copy)]
pub struct RegisterProjector;

impl Projector for RegisterProjector {
    type Event = RegisterEvent;
    type State = RegisterView;
    type Error = ViewError;

    fn initial(&self) -> RegisterView {
        RegisterView::default()
    }

    /// Keyless fold: this projector cannot attribute an event without the
    /// origin stream key, so the required `apply` is its error path.
    fn apply(
        &self,
        _view: RegisterView,
        _event: &RegisterEvent,
    ) -> Result<RegisterView, ViewError> {
        Err(ViewError::Unattributed)
    }

    fn apply_attributed(
        &self,
        mut view: RegisterView,
        stream_key: Option<&StreamKey>,
        event: &RegisterEvent,
    ) -> Result<RegisterView, ViewError> {
        let id = stream_key.ok_or(ViewError::Unattributed).and_then(|k| {
            RegisterId::from_key_bytes(k.as_bytes()).ok_or(ViewError::BadStreamKey)
        })?;
        match event {
            RegisterEvent::Inception { owner_pubkey, sig } => {
                // Content-addressing check: the stream id must equal blake3 of
                // the key that signs the genesis event.
                if RegisterId::from_pubkey(owner_pubkey) != id {
                    return Err(ViewError::IdMismatch);
                }
                if view.chains.contains_key(&id) {
                    return Err(ViewError::AlreadyIncepted);
                }
                let owner =
                    VerifyingKey::from_bytes(owner_pubkey).map_err(|_| ViewError::BadSignature)?;
                owner
                    .verify_strict(
                        &inception_preimage(owner_pubkey),
                        &Signature::from_bytes(sig),
                    )
                    .map_err(|_| ViewError::BadSignature)?;
                view.registers.insert(id, HashMap::new());
                view.chains.insert(
                    id,
                    ChainHead {
                        owner,
                        last_digest: event_digest(event),
                    },
                );
            }
            RegisterEvent::Set {
                key,
                val,
                prior_digest,
                sig,
            } => {
                let head = *view.chains.get(&id).ok_or(ViewError::NotIncepted)?;
                // Chain first, so a wrong link is BrokenChain, not BadSignature.
                if *prior_digest != head.last_digest {
                    return Err(ViewError::BrokenChain {
                        expected: head.last_digest,
                        actual: *prior_digest,
                    });
                }
                head.owner
                    .verify_strict(
                        &set_preimage(key, val, prior_digest),
                        &Signature::from_bytes(sig),
                    )
                    .map_err(|_| ViewError::BadSignature)?;
                let entries = view.registers.get_mut(&id).ok_or(ViewError::NotIncepted)?;
                entries.insert(key.clone(), val.clone());
                view.chains.insert(
                    id,
                    ChainHead {
                        owner: head.owner,
                        last_digest: event_digest(event),
                    },
                );
            }
        }
        Ok(view)
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
    use super::{RegisterProjector, RegisterView, ViewError};
    use crate::domain::{
        Incept, RegisterEvent, RegisterId, SignedRegister, SubmitSet, event_digest, set_preimage,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use mnesis::{AggregateRoot, Handle, Version};
    use mnesis_store::{Projector, StreamKey};
    use rand_core::OsRng;

    /// Build a genuine `(inception, set)` chain for one register via the real
    /// aggregate `Handle` path, returning the key, id, and the two events.
    fn genuine_chain() -> (SigningKey, RegisterId, RegisterEvent, RegisterEvent) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let id = RegisterId::from_pubkey(&signing_key.verifying_key().to_bytes());
        let mut root: AggregateRoot<SignedRegister> = SignedRegister::new(id);

        let e1 = SignedRegister::handle(
            root.state(),
            Incept {
                signing_key: signing_key.clone(),
            },
        )
        .unwrap()
        .unwrap();
        let inception = e1.first().clone();
        root.commit_persisted(Version::INITIAL, &e1);

        let e2 = SignedRegister::handle(
            root.state(),
            SubmitSet {
                key: "a".to_owned(),
                val: "1".to_owned(),
                signing_key: signing_key.clone(),
            },
        )
        .unwrap()
        .unwrap();
        let set = e2.first().clone();

        (signing_key, id, inception, set)
    }

    fn fold(
        view: RegisterView,
        id: RegisterId,
        event: &RegisterEvent,
    ) -> Result<RegisterView, ViewError> {
        let key = StreamKey::from_slice(id.as_ref());
        RegisterProjector.apply_attributed(view, Some(&key), event)
    }

    #[test]
    fn folds_a_genuine_chain_into_the_view() {
        let (_sk, id, inception, set) = genuine_chain();
        let mut view = RegisterProjector.initial();
        view = fold(view, id, &inception).expect("inception verifies");
        view = fold(view, id, &set).expect("set verifies");
        assert_eq!(
            view.entries_of(&id).and_then(|e| e.get("a")),
            Some(&"1".to_owned())
        );
    }

    #[test]
    fn rejects_inception_stored_under_the_wrong_id() {
        let (_sk, _id, inception, _set) = genuine_chain();
        let wrong = RegisterId::from_pubkey(&[7u8; 32]);
        assert_eq!(
            fold(RegisterProjector.initial(), wrong, &inception).unwrap_err(),
            ViewError::IdMismatch
        );
    }

    #[test]
    fn rejects_a_tampered_signature() {
        let (_sk, id, inception, set) = genuine_chain();
        let view = fold(RegisterProjector.initial(), id, &inception).unwrap();
        let mut tampered = set;
        if let RegisterEvent::Set { sig, .. } = &mut tampered {
            sig[0] ^= 0xff;
        }
        assert_eq!(
            fold(view, id, &tampered).unwrap_err(),
            ViewError::BadSignature
        );
    }

    #[test]
    fn rejects_a_broken_chain_link() {
        let (signing_key, id, inception, _set) = genuine_chain();
        let view = fold(RegisterProjector.initial(), id, &inception).unwrap();
        let genuine_head = event_digest(&inception);

        // Craft a Set whose prior_digest is wrong but is validly signed by the
        // owner over that wrong preimage — isolates the chain check from the
        // signature check.
        let wrong_prior = [0xAB; 32];
        let preimage = set_preimage("a", "1", &wrong_prior);
        let sig = signing_key.sign(&preimage).to_bytes();
        let bad = RegisterEvent::Set {
            key: "a".to_owned(),
            val: "1".to_owned(),
            prior_digest: wrong_prior,
            sig,
        };
        assert_eq!(
            fold(view, id, &bad).unwrap_err(),
            ViewError::BrokenChain {
                expected: genuine_head,
                actual: wrong_prior,
            }
        );
    }

    #[test]
    fn rejects_a_forged_event_from_a_non_owner_key() {
        let (_sk, id, inception, _set) = genuine_chain();
        let view = fold(RegisterProjector.initial(), id, &inception).unwrap();
        let head = event_digest(&inception);

        // An attacker signs a well-chained Set with their own key — the chain
        // link is correct, but the signature does not verify against the owner.
        let attacker = SigningKey::generate(&mut OsRng);
        let preimage = set_preimage("a", "evil", &head);
        let sig = attacker.sign(&preimage).to_bytes();
        let forged = RegisterEvent::Set {
            key: "a".to_owned(),
            val: "evil".to_owned(),
            prior_digest: head,
            sig,
        };
        assert_eq!(
            fold(view, id, &forged).unwrap_err(),
            ViewError::BadSignature
        );
    }

    #[test]
    fn rejects_a_set_before_inception() {
        let (_sk, id, _inception, set) = genuine_chain();
        assert_eq!(
            fold(RegisterProjector.initial(), id, &set).unwrap_err(),
            ViewError::NotIncepted
        );
    }

    #[test]
    fn keyless_apply_is_unattributed() {
        let (_sk, _id, inception, _set) = genuine_chain();
        assert_eq!(
            RegisterProjector
                .apply(RegisterProjector.initial(), &inception)
                .unwrap_err(),
            ViewError::Unattributed
        );
    }

    #[test]
    fn rejects_a_stream_key_that_is_not_a_register_id() {
        let (_sk, _id, inception, _set) = genuine_chain();
        let short = StreamKey::from_slice(b"too-short");
        assert_eq!(
            RegisterProjector
                .apply_attributed(RegisterProjector.initial(), Some(&short), &inception)
                .unwrap_err(),
            ViewError::BadStreamKey
        );
    }
}
