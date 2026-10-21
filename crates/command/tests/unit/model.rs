#![allow(clippy::unwrap_used, reason = "test failures retain diagnostics")]

use std::convert::Infallible;
use std::num::{NonZeroU64, NonZeroUsize};

use mnesis::{
    Aggregate, AggregateRoot, AggregateState, DomainEvent, Events, Handle, KernelError, Message,
    Version, events,
};
use mnesis_store::{Decode, Encode, PersistedEnvelope, SchemaVersion};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    Acceptance, AcceptanceCodec, Adapted, Codec, CodecError, Decision, DecisionError, Mismatch,
    Record, Request, State,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Fact(u8);
impl Message for Fact {}
impl DomainEvent for Fact {
    fn name(&self) -> &'static str {
        "fact"
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ValueState(u64);
impl AggregateState for ValueState {
    type Event = Fact;
    fn initial() -> Self {
        Self(0)
    }
    fn apply(self, fact: &Fact) -> Self {
        Self(self.0 + u64::from(fact.0))
    }
}
struct Fixture;
impl Aggregate for Fixture {
    type State = ValueState;
    type Error = Infallible;
    type Id = String;
    const MAX_REHYDRATION_EVENTS: NonZeroUsize = NonZeroUsize::MIN.saturating_add(7);
}
impl Handle<u8> for Fixture {
    fn handle(_: &ValueState, command: u8) -> Result<Option<Events<Fact>>, Infallible> {
        Ok((command != 0).then(|| events![Fact(command)]))
    }
}
type TestState = State<Fixture, u8, NonZeroU64, 0>;
type TestRecord = Record<u8, NonZeroU64, Fact>;
type ReadAcceptance = Acceptance<u8, Fact>;

fn accepted() -> Value {
    json!({"schema":2,"request":{"id":1,"expected":null,"command":7},
        "receipt":{"id":1,"version":1,"events":1},"facts":[7]})
}
fn envelope(kind: &str, value: &Value) -> PersistedEnvelope {
    raw_envelope(
        kind,
        &serde_json::to_vec(value).unwrap(),
        SchemaVersion::INITIAL,
    )
}
fn raw_envelope(kind: &str, payload: &[u8], schema: SchemaVersion) -> PersistedEnvelope {
    versioned_envelope(Version::INITIAL, kind, payload, schema)
}
fn versioned_envelope(
    version: Version,
    kind: &str,
    payload: &[u8],
    schema: SchemaVersion,
) -> PersistedEnvelope {
    let mut bytes = kind.as_bytes().to_vec();
    let boundary = u32::try_from(bytes.len()).unwrap();
    bytes.extend_from_slice(payload);
    let end = u32::try_from(bytes.len()).unwrap();
    PersistedEnvelope::try_new(
        version,
        bytes.into(),
        schema,
        0..boundary,
        boundary..end,
        None,
    )
    .unwrap()
}

#[test]
fn original_writer_records_replay_and_new_writes_preserve_rollback_bytes() {
    let mut root = AggregateRoot::<Adapted<Fixture, u8, NonZeroU64, 0>>::new("golden".into());
    for (number, command, count, bytes) in [
        (
            1,
            7,
            1,
            include_bytes!("../fixtures/counter-v2.json").as_slice(),
        ),
        (
            2,
            0,
            0,
            include_bytes!("../fixtures/noop-v2.json").as_slice(),
        ),
    ] {
        let version = Version::new(number).unwrap();
        let decoded: ReadAcceptance = AcceptanceCodec
            .decode(&versioned_envelope(
                version,
                "controller.accepted.v2",
                bytes,
                SchemaVersion::INITIAL,
            ))
            .unwrap();
        let original = Request {
            id: NonZeroU64::new(number).unwrap(),
            expected: Version::new(number - 1),
            command,
        };
        let receipt = super::Receipt {
            id: original.id,
            version,
            events: count,
        };
        assert_eq!(decoded.request(), &original);
        assert_eq!(decoded.receipt(), &receipt);
        assert_eq!(
            decoded.facts(),
            if count == 0 { &[][..] } else { &[Fact(7)][..] }
        );
        assert_eq!(decoded.name(), "controller.accepted.v2");
        let decision = Decision {
            request: original.clone(),
            version,
        };
        let written =
            Adapted::<Fixture, u8, NonZeroU64, 0>::handle(root.state().unwrap(), decision)
                .unwrap()
                .unwrap();
        let record = written.first();
        assert_eq!(Codec.encode(record).unwrap().as_ref(), bytes);
        root.replay(version, &decoded.0).unwrap();
        assert_eq!(root.version(), Some(version));
        assert_eq!(
            root.state().unwrap().receipt(&original).unwrap(),
            Some(receipt)
        );
        assert_eq!(root.state().unwrap().domain, ValueState(7));
        assert!(root.state().unwrap().integrity.is_ok());
    }
}

#[test]
fn invalid_acceptance_metadata_is_rejected_at_the_codec_boundary() {
    let original = accepted();
    let decoded: ReadAcceptance = AcceptanceCodec
        .decode(&envelope("controller.accepted.v2", &original))
        .unwrap();
    let replayed = TestState::initial().apply(&decoded.0);
    assert_eq!(replayed.domain, ValueState(7));
    for (path, replacement) in [
        (vec!["schema"], json!(1)),
        (vec!["schema"], json!(3)),
        (vec!["request", "expected"], json!(1)),
        (vec!["receipt", "id"], json!(2)),
        (vec!["receipt", "version"], json!(2)),
        (vec!["receipt", "events"], json!(0)),
    ] {
        let mut changed = original.clone();
        let mut target = &mut changed;
        for key in path {
            target = &mut target[key];
        }
        *target = replacement;
        let result: Result<ReadAcceptance, _> =
            AcceptanceCodec.decode(&envelope("controller.accepted.v2", &changed));
        assert!(
            matches!(result, Err(CodecError::Acceptance)),
            "wrong result: {changed}"
        );
    }
    for kind in ["controller.accepted.v1", "controller.accepted.v3", "other"] {
        let unsupported: Result<ReadAcceptance, _> =
            AcceptanceCodec.decode(&envelope(kind, &original));
        assert!(matches!(unsupported, Err(CodecError::Envelope)));
    }
}

#[test]
fn duplicate_unknown_truncated_and_invalid_json_never_create_public_acceptance() {
    let original = std::str::from_utf8(include_bytes!("../fixtures/counter-v2.json")).unwrap();
    for payload in [
        original.replace("\"schema\":2,", "\"schema\":2,\"schema\":2,"),
        original.replace("\"request\":{\"id\":1,", "\"request\":{\"id\":1,\"id\":1,"),
        original.replace("\"receipt\":{\"id\":1,", "\"receipt\":{\"id\":1,\"id\":1,"),
        original.replace("\"schema\":2,", "\"schema\":2,\"extra\":0,"),
        original.replace("\"command\":7", "\"command\":7,\"extra\":0"),
        original.replace("\"events\":1", "\"events\":1,\"extra\":0"),
        original.replace("\"request\":{\"id\":1", "\"request\":{\"id\":0"),
        original.replace("\"expected\":null", "\"expected\":0"),
        original.replace("\"command\":7", "\"command\":\"bad\""),
        original[..original.len() - 1].into(),
    ] {
        assert_ne!(payload, original);
        let result: Result<ReadAcceptance, _> = AcceptanceCodec.decode(&raw_envelope(
            "controller.accepted.v2",
            payload.as_bytes(),
            SchemaVersion::INITIAL,
        ));
        assert!(
            matches!(result, Err(CodecError::Json(_))),
            "wrong result: {payload}"
        );
    }
}

#[test]
fn ordered_replay_rejects_a_gap_and_cumulative_facts_without_applying_them() {
    let mut root = AggregateRoot::<Adapted<Fixture, u8, NonZeroU64, 0>>::new("cumulative".into());
    for number in 1..=9 {
        let mut value = accepted();
        value["request"]["id"] = json!(number);
        value["request"]["expected"] = json!(Version::new(number - 1).map(Version::as_u64));
        value["receipt"]["id"] = json!(number);
        value["receipt"]["version"] = json!(number);
        value["facts"] = json!([1]);
        let version = Version::new(number).unwrap();
        let record: ReadAcceptance = AcceptanceCodec
            .decode(&versioned_envelope(
                version,
                "controller.accepted.v2",
                &serde_json::to_vec(&value).unwrap(),
                SchemaVersion::INITIAL,
            ))
            .unwrap();
        if number == 2 {
            let mut empty = AggregateRoot::<Adapted<Fixture, u8, NonZeroU64, 0>>::new("gap".into());
            assert!(
                matches!(empty.replay(version, &record.0), Err(KernelError::VersionMismatch {
                expected: Version::INITIAL, actual
            }) if actual == version)
            );
            assert_eq!(empty.version(), None);
            assert_eq!(empty.state().unwrap().domain, ValueState(0));
        }
        root.replay(version, &record.0).unwrap();
        let state = root.state().unwrap();
        assert_eq!(state.domain, ValueState(number.min(8)));
        assert_eq!(state.facts, usize::try_from(number.min(8)).unwrap());
        if number <= 8 {
            assert!(state.integrity.is_ok());
            assert_eq!(
                state.receipt(record.request()).unwrap(),
                Some(record.receipt().clone())
            );
        } else {
            assert!(state.integrity.is_err());
            assert_eq!(state.receipt(record.request()).unwrap(), None);
        }
    }
}

#[test]
fn replay_rejects_excess_facts_and_duplicate_id_without_changing_domain_state() {
    let record: TestRecord = serde_json::from_value(accepted()).unwrap();
    let once = TestState::initial().apply(&record);
    let mut duplicate = accepted();
    duplicate["request"]["expected"] = json!(1);
    duplicate["receipt"]["version"] = json!(2);
    let repeated = serde_json::from_value(duplicate).unwrap();
    let twice = once.apply(&repeated);
    assert_eq!(twice.domain, ValueState(7));
    assert!(twice.integrity.is_err());
    let mut later = accepted();
    later["request"]["id"] = json!(3);
    later["request"]["expected"] = json!(2);
    later["receipt"]["id"] = json!(3);
    later["receipt"]["version"] = json!(3);
    let subsequent = twice.apply(&serde_json::from_value(later).unwrap());
    assert!(subsequent.integrity.is_err());
    assert_eq!(subsequent.domain, ValueState(7));
    let mut overflow = accepted();
    overflow["facts"] = json!([7, 9]);
    overflow["receipt"]["events"] = json!(2);
    let decoded: TestRecord = Codec
        .decode(&envelope("controller.accepted.v2", &overflow))
        .unwrap();
    let refused = TestState::initial().apply(&decoded);
    assert_eq!(refused.domain, ValueState(0));
    assert!(refused.integrity.is_err());
}

#[test]
fn full_receipt_history_preserves_old_ids_including_noops() {
    assert_eq!(crate::CommandProcessor::<Fixture, u8>::capacity(), 4096);
    let mut root = AggregateRoot::<Adapted<Fixture, u8, NonZeroU64, 0>>::new(String::from("full"));
    for ordinal in 1..=4096 {
        let record = Record {
            schema: 2,
            request: Request {
                id: NonZeroU64::new(ordinal).unwrap(),
                expected: Version::new(ordinal - 1),
                command: 0,
            },
            receipt: super::Receipt {
                id: NonZeroU64::new(ordinal).unwrap(),
                version: Version::new(ordinal).unwrap(),
                events: 0,
            },
            facts: Vec::new(),
        };
        root.replay(Version::new(ordinal).unwrap(), &record)
            .unwrap();
    }
    let state = root.state().unwrap();
    assert!(state.integrity.is_ok());
    assert_eq!(state.domain, ValueState(0));
    let old = Request {
        id: NonZeroU64::MIN,
        expected: None,
        command: 0,
    };
    let receipt = state.receipt(&old).unwrap().unwrap();
    assert_eq!((receipt.version, receipt.events), (Version::INITIAL, 0));
    let decision = Decision {
        request: Request {
            id: NonZeroU64::new(4097).unwrap(),
            expected: Version::new(4096),
            command: 0,
        },
        version: Version::new(4097).unwrap(),
    };
    let rejected = Adapted::<Fixture, u8, NonZeroU64, 0>::handle(state, decision);
    assert!(matches!(rejected, Err(DecisionError::HistoryFull)));
    let changed = Request { command: 7, ..old };
    assert!(matches!(state.receipt(&changed), Err(Mismatch)));
    replay_overflow(root);
}

fn replay_overflow(mut root: AggregateRoot<Adapted<Fixture, u8, NonZeroU64, 0>>) {
    let excess = Record {
        schema: 2,
        request: Request {
            id: NonZeroU64::new(4097).unwrap(),
            expected: Version::new(4096),
            command: 0,
        },
        receipt: super::Receipt {
            id: NonZeroU64::new(4097).unwrap(),
            version: Version::new(4097).unwrap(),
            events: 0,
        },
        facts: Vec::new(),
    };
    let overflow = root.replay(Version::new(4097).unwrap(), &excess);
    assert!(matches!(
        overflow,
        Err(KernelError::RehydrationLimitExceeded { max: 4096 })
    ));
    assert_eq!(root.version(), Version::new(4096));
}

#[test]
fn envelope_schema_and_exact_payload_limit_are_enforced_independently() {
    let mut payload = serde_json::to_vec(&accepted()).unwrap();
    payload.resize(4096, b' ');
    let edge: Result<ReadAcceptance, _> = AcceptanceCodec.decode(&raw_envelope(
        "controller.accepted.v2",
        &payload,
        SchemaVersion::INITIAL,
    ));
    assert!(edge.is_ok());
    payload.push(b' ');
    let exceeded: Result<ReadAcceptance, _> = AcceptanceCodec.decode(&raw_envelope(
        "controller.accepted.v2",
        &payload,
        SchemaVersion::INITIAL,
    ));
    assert!(matches!(exceeded, Err(CodecError::Bytes { source: None })));
    let wrong_schema: Result<ReadAcceptance, _> = AcceptanceCodec.decode(&raw_envelope(
        "controller.accepted.v2",
        &serde_json::to_vec(&accepted()).unwrap(),
        SchemaVersion::from_u32(2).unwrap(),
    ));
    assert!(matches!(wrong_schema, Err(CodecError::Envelope)));
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Lossy {
    kept: u8,
    #[serde(skip)]
    lost: u8,
}

#[test]
fn lossy_request_and_fact_serialization_cannot_be_accepted() {
    let request = Request {
        id: NonZeroU64::MIN,
        expected: None,
        command: Lossy { kept: 1, lost: 2 },
    };
    let rejected = request.validate();
    assert!(matches!(rejected, Err(CodecError::Lossy)));
    let record = Record {
        schema: 2,
        request: Request {
            id: NonZeroU64::MIN,
            expected: None,
            command: 1_u8,
        },
        receipt: super::Receipt {
            id: NonZeroU64::MIN,
            version: Version::INITIAL,
            events: 1,
        },
        facts: vec![Lossy { kept: 3, lost: 4 }],
    };
    let encoded = Codec.encode(&record);
    assert!(matches!(encoded, Err(CodecError::Lossy)));
}

#[test]
fn bounded_request_bytes_accept_the_limit_and_reject_the_next_byte() {
    for prefix in ["", "é", "🦀", "\0\n\\\""] {
        let empty = Request {
            id: NonZeroU64::MIN,
            expected: None,
            command: prefix.to_owned(),
        };
        let overhead = empty.encode().unwrap().len();
        let edge = Request {
            command: format!("{prefix}{}", "x".repeat(4096 - overhead)),
            ..empty
        };
        let mut below = edge.clone();
        assert_eq!(below.command.pop(), Some('x'));
        assert_eq!(below.encode().unwrap().len(), 4095);
        let encoded = edge.encode().unwrap();
        assert_eq!(encoded.len(), 4096);
        assert_eq!(encoded, serde_json::to_vec(&edge).unwrap());
        let decoded = Request::<String>::decode(&encoded).unwrap();
        assert_eq!(decoded, edge);
        let too_big = Request {
            command: format!("{}x", edge.command),
            ..edge
        };
        assert!(
            matches!(too_big.encode(), Err(CodecError::Bytes { source: Some(error) }) if error.io_error_kind() == Some(std::io::ErrorKind::WriteZero))
        );
        let mut oversized = encoded;
        oversized.push(b' ');
        assert!(matches!(
            Request::<String>::decode(&oversized),
            Err(CodecError::Bytes { source: None })
        ));
    }
}
