//! One bounded owner keeps the native view alive independently of caller polling.

use std::collections::VecDeque;
use std::future::Future;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::{BoxStream, try_unfold};
use mnesis::{ErrorId, Version};
use mnesis_store::export::{ConsistentExporter, ExportError, ExportSession};
use mnesis_store::{PersistedEnvelope, StreamKey};
use sqlx::pool::PoolConnection;
use sqlx::{Connection, PgConnection, Postgres};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout_at};

use super::listing::{self, Key};
use super::{EventRow, PostgresStore, read_key, row_to_envelope, stream_query, validate_bytes};
use crate::page::PageRow;
use crate::{Limits, PostgresError};

type Error = ExportError<PostgresError>;
type Reply<T> = oneshot::Sender<Result<T, PostgresError>>;
type Rows<T> = VecDeque<PageRow<T>>;

enum Request {
    Key {
        id: StreamKey,
        reply: Reply<Option<i64>>,
    },
    List {
        after: Option<i64>,
        reply: Reply<Rows<Key>>,
    },
    Events {
        key: i64,
        from: Version,
        reply: Reply<Rows<EventRow>>,
    },
}

#[derive(Clone, Copy)]
struct Window {
    deadline: Instant,
    limits: Limits,
}

/// A finite read-only view; cursors borrow it and close waits for its owner.
pub struct Session {
    requests: mpsc::Sender<Request>,
    stop: oneshot::Sender<()>,
    owner: JoinHandle<Result<(), PostgresError>>,
    window: Window,
}

impl Window {
    fn live(&self) -> Result<(), Error> {
        if Instant::now() >= self.deadline {
            Err(ExportError::Expired)
        } else {
            Ok(())
        }
    }

    fn unavailable(&self, source: PostgresError) -> Error {
        if Instant::now() >= self.deadline {
            ExportError::Expired
        } else {
            ExportError::Store(source)
        }
    }
}

impl Session {
    async fn request<T: Send>(
        &self,
        make: impl FnOnce(Reply<T>) -> Request + Send,
    ) -> Result<T, Error> {
        self.window.live()?;
        let (reply, response) = oneshot::channel();
        let permit = self.requests.reserve().await.map_err(|source| {
            self.window
                .unavailable(PostgresError::ExportRequest(source))
        })?;
        permit.send(make(reply));
        response
            .await
            .map_err(|source| self.window.unavailable(PostgresError::ExportReply(source)))?
            .map_err(ExportError::Store)
    }
}

impl ConsistentExporter for PostgresStore {
    type Session<'a> = Session;

    async fn open_export_session(&self, lifetime: Duration) -> Result<Session, Error> {
        if lifetime.is_zero() || Instant::now().checked_add(lifetime).is_none() {
            return Err(ExportError::InvalidLifetime { lifetime });
        }
        let mut connection = self
            .pool()
            .acquire()
            .await
            .map_err(PostgresError::Sqlx)
            .map_err(ExportError::Store)?;
        connection.close_on_drop();
        let deadline = Instant::now()
            .checked_add(lifetime)
            .ok_or(ExportError::InvalidLifetime { lifetime })?;
        let limits = self.inner.limits;
        let (requests, receive) = mpsc::channel(1);
        let (stop, canceled) = oneshot::channel();
        let (ready, started) = oneshot::channel();
        let window = Window { deadline, limits };
        let closed = self.pool().close_event();
        let stopped = async move {
            tokio::select! {
                _ = canceled => {},
                () = closed => {},
            }
        };
        let owner = tokio::spawn(own(connection, receive, stopped, ready, window));
        let session = Session {
            requests,
            stop,
            owner,
            window,
        };
        started
            .await
            .map_err(|source| {
                session
                    .window
                    .unavailable(PostgresError::ExportReply(source))
            })?
            .map_err(ExportError::Store)?;
        Ok(session)
    }
}

async fn own(
    mut connection: PoolConnection<Postgres>,
    requests: mpsc::Receiver<Request>,
    stopped: impl Future<Output = ()> + Send,
    ready: Reply<()>,
    window: Window,
) -> Result<(), PostgresError> {
    let result = tokio::select! {
        () = stopped => Ok(()),
        completed = timeout_at(window.deadline, serve(&mut connection, requests, ready, window.limits)) => {
            match completed {
                Ok(result) => result,
                // Expiry is normal view termination. Requests report Expired;
                // native connection cleanup still completes below.
                Err(_elapsed) => Ok(()),
            }
        }
    };
    let closed = connection.close().await.map_err(PostgresError::Sqlx);
    result.and(closed)
}

async fn serve(
    connection: &mut PgConnection,
    mut requests: mpsc::Receiver<Request>,
    ready: Reply<()>,
    limits: Limits,
) -> Result<(), PostgresError> {
    let opened = async {
        let mut tx = connection
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await?;
        // Socket closure alone leaves a lock-waiting server query alive.
        // Keep a caller's shorter check interval and scope this to the view.
        sqlx::query_scalar::<_, String>(
            "SELECT pg_catalog.set_config('client_connection_check_interval', \
             LEAST(COALESCE(NULLIF(setting::bigint, 0), $1), $1)::text, true) \
             FROM pg_catalog.pg_settings WHERE name = 'client_connection_check_interval'",
        )
        .bind(1000_i64)
        .fetch_one(&mut *tx)
        .await?;
        let captured: String = sqlx::query_scalar("SELECT pg_catalog.pg_current_snapshot()::text")
            .fetch_one(&mut *tx)
            .await?;
        Ok::<_, sqlx::Error>((tx, captured))
    }
    .await;
    let (mut tx, captured) = match opened {
        Ok(value) => value,
        Err(source) => {
            drop(ready.send(Err(PostgresError::Sqlx(source))));
            return Ok(());
        }
    };
    if ready.send(Ok(())).is_err() {
        return tx.rollback().await.map_err(PostgresError::Sqlx);
    }
    while let Some(request) = requests.recv().await {
        // A canceled cursor abandons its reply, never a write or the fixed view.
        match request {
            Request::Key { id, reply } => {
                drop(reply.send(read_key(&mut tx, &id).await));
            }
            Request::List { after, reply } => {
                drop(reply.send(listing::page(&mut *tx, after, &captured, limits).await));
            }
            Request::Events { key, from, reply } => {
                let mut query = stream_query(
                    key,
                    &from.as_u64().to_string(),
                    &u64::MAX.to_string(),
                    None,
                    limits,
                );
                let result = query
                    .build_query_as::<PageRow<EventRow>>()
                    .fetch_all(&mut *tx)
                    .await
                    .map(VecDeque::from)
                    .map_err(PostgresError::Sqlx);
                drop(reply.send(result));
            }
        }
    }
    tx.rollback().await.map_err(PostgresError::Sqlx)
}

impl ExportSession for Session {
    type Error = PostgresError;
    type StreamList<'a> = BoxStream<'a, Result<StreamKey, Error>>;
    type ExportStream<'a> = BoxStream<'a, Result<PersistedEnvelope, Error>>;

    async fn list_streams(&self) -> Result<Self::StreamList<'_>, Error> {
        self.window.live()?;
        Ok(try_unfold(
            (None, VecDeque::new()),
            move |(after, mut rows)| async move {
                self.window.live()?;
                if rows.is_empty() {
                    rows = self.request(|reply| Request::List { after, reply }).await?;
                }
                let Some(buffered) = rows.pop_front() else {
                    return Ok(None);
                };
                let key = buffered.into_value().map_err(ExportError::Store)?;
                Ok(Some((
                    StreamKey::from_bytes(key.value),
                    (Some(key.number), rows),
                )))
            },
        )
        .boxed())
    }

    async fn export_stream(
        &self,
        id: &StreamKey,
        from: Version,
    ) -> Result<Self::ExportStream<'_>, Error> {
        self.window.live()?;
        validate_bytes(&[id.as_bytes().len()], self.window.limits).map_err(ExportError::Store)?;
        let label = ErrorId::from_display(id);
        let key = self
            .request(|reply| Request::Key {
                id: id.clone(),
                reply,
            })
            .await?;
        Ok(try_unfold(
            (Some(from), VecDeque::new()),
            move |(lower, mut rows)| async move {
                self.window.live()?;
                let Some(number) = key else { return Ok(None) };
                if rows.is_empty() {
                    let Some(position) = lower else {
                        return Ok(None);
                    };
                    rows = self
                        .request(|reply| Request::Events {
                            key: number,
                            from: position,
                            reply,
                        })
                        .await?;
                }
                let Some(buffered) = rows.pop_front() else {
                    return Ok(None);
                };
                let event = buffered
                    .into_value()
                    .and_then(|row| row_to_envelope(row, label))
                    .map_err(ExportError::Store)?;
                let next = event.version().next();
                Ok(Some((event, (next, rows))))
            },
        )
        .boxed())
    }

    async fn close(self) -> Result<(), Error> {
        let Self {
            requests,
            stop,
            owner,
            ..
        } = self;
        drop(requests);
        // Keep cancellation live until the ordinary rollback is acknowledged.
        let completed = owner
            .await
            .map_err(PostgresError::ExportTask)
            .map_err(ExportError::Store)?;
        drop(stop);
        completed.map_err(ExportError::Store)
    }
}
