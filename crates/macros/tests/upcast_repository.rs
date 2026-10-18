//! Generated schema lookup and rename dispatch through a real repository.
#![allow(clippy::unwrap_used, reason = "exact repository regression assertions")]

use std::convert::Infallible;
use std::fmt;

use futures::TryStreamExt;
use mnesis::{AggregateState, Version, events};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{RawEventStore, SchemaVersion, StreamKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, mnesis::DomainEvent)]
enum Event {
    Old(u32),
    New(u32),
}

#[derive(Debug, Default)]
struct State(Vec<Event>);
impl AggregateState for State {
    type Event = Event;
    fn initial() -> Self {
        Self::default()
    }
    fn apply(mut self, event: &Event) -> Self {
        self.0.push(event.clone());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Id([u8; 1]);
impl AsRef<[u8]> for Id {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0[0])
    }
}

#[mnesis::aggregate(state = State, error = Infallible, id = Id)]
struct Model;

#[derive(Deserialize)]
enum Legacy {
    Old(u32),
}
#[derive(Serialize, Deserialize)]
struct Intermediate {
    value: u32,
}

#[mnesis_macros::transforms(aggregate = Model, error = serde_json::Error)]
impl Migrations {
    #[transform(event = "Old", from = 1, to = 2, rename = "New")]
    fn rename(payload: &[u8]) -> Result<Vec<u8>, serde_json::Error> {
        let Legacy::Old(value) = serde_json::from_slice(payload)?;
        serde_json::to_vec(&Intermediate { value })
    }
    #[transform(event = "New", from = 2, to = 3)]
    fn latest(payload: &[u8]) -> Result<Vec<u8>, serde_json::Error> {
        let intermediate: Intermediate = serde_json::from_slice(payload)?;
        serde_json::to_vec(&Event::New(intermediate.value))
    }
}

#[test]
fn generated_schema_lookup_matches_actual_wire_names_and_replayed_shapes() {
    futures::executor::block_on(async {
        let store = InMemoryStore::new().into_store();
        let repo = store.repository::<Model>().json().build();
        for (key, event, name, schema, canonical) in [
            (1, Event::Old(3), "Old", 1, Event::New(3)),
            (2, Event::New(4), "New", 3, Event::New(4)),
        ] {
            let id = Id([key]);
            let mut root = Model::new(id.clone());
            repo.save_with::<_, 0>(
                &mut root,
                &events![event.clone()],
                Migrations::current_version,
            )
            .await
            .unwrap();
            assert_eq!(root.version(), Some(Version::INITIAL));
            assert_eq!(root.state().unwrap().0, vec![event]);
            let rows: Vec<_> = store
                .read_stream(&StreamKey::from_slice(id.as_ref()), Version::INITIAL)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].event_type(), name);
            assert_eq!(rows[0].schema_version(), schema);
            let loaded = repo
                .load_with(
                    id,
                    move |envelope| {
                        assert_eq!(envelope.event_type(), name);
                        assert_eq!(
                            envelope.schema_version_value(),
                            SchemaVersion::from_u32(schema).unwrap()
                        );
                        Ok::<_, Infallible>(())
                    },
                    Migrations::upcast,
                )
                .await
                .unwrap();
            assert_eq!(loaded.version(), Some(Version::INITIAL));
            assert_eq!(loaded.state().unwrap().0, vec![canonical]);
        }
    });
}

#[test]
fn undeclared_schemas_of_known_names_are_rejected_on_real_rows() {
    futures::executor::block_on(async {
        let store = InMemoryStore::new().into_store();
        let repo = store.repository::<Model>().json().build();
        for (key, name, schema, event) in [
            (3, "Old", 2, Event::Old(5)),
            (4, "New", u32::MAX, Event::New(6)),
        ] {
            let id = Id([key]);
            let envelope = mnesis_store::pending_envelope(Version::INITIAL)
                .event_type(name)
                .payload(serde_json::to_vec(&event).unwrap())
                .schema_version(SchemaVersion::from_u32(schema).unwrap())
                .build()
                .unwrap();
            store
                .append(
                    &StreamKey::from_slice(id.as_ref()),
                    None,
                    mnesis_store::PendingBatch::new(&[envelope]).unwrap(),
                )
                .await
                .unwrap();
            let result = repo
                .load_with(id, |_| Ok::<_, Infallible>(()), Migrations::upcast)
                .await;
            let error = result.err().unwrap();
            match error {
                mnesis_store::LoadWithError::Upcast(
                    mnesis_store::TransformError::UnsupportedSchema {
                        event_type,
                        schema: rejected,
                    },
                ) => {
                    assert_eq!(event_type.to_string(), name);
                    assert_eq!(rejected, SchemaVersion::from_u32(schema).unwrap());
                }
                other => panic!("expected a schema rejection, got {other:?}"),
            }
        }
    });
}
