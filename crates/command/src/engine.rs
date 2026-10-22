use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use mnesis::{AggregateRoot, EventOf, Handle, KernelError, Version};
use mnesis_store::{
    CommandRepository, EventStore, ExecuteError, RawEventStore, Repository, Store, StoreError,
};
use parking_lot::Mutex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use crate::model::{Adapted, Codec, Decision, Mismatch};
use crate::trace;
use crate::{Cause, CodecError, Error, Receipt, Request};

type Root<A, C, I, const N: usize> = AggregateRoot<Adapted<A, C, I, N>>;
pub type Handoff<C, I> = Arc<Mutex<Option<Request<C, I>>>>;

pub struct Engine<S, A, C, I, const N: usize>
where
    A: Handle<C, N> + 'static,
    C: Clone + Debug + Eq + Send + Sync + 'static,
    I: Clone + Debug + Ord + Send + Sync + 'static,
    EventOf<A>: Clone,
{
    repository: EventStore<S, Codec, Adapted<A, C, I, N>>,
    id: A::Id,
    timeout: Duration,
    pending: Option<Request<C, I>>,
    handoff: Handoff<C, I>,
}

impl<S, A, C, I, const N: usize> Drop for Engine<S, A, C, I, N>
where
    A: Handle<C, N> + 'static,
    C: Clone + Debug + Eq + Send + Sync + 'static,
    I: Clone + Debug + Ord + Send + Sync + 'static,
    EventOf<A>: Clone,
{
    fn drop(&mut self) {
        let previous = std::mem::replace(&mut *self.handoff.lock(), self.pending.take());
        drop(previous);
    }
}

impl<S, A, C, I, const N: usize> Engine<S, A, C, I, N>
where
    S: RawEventStore + 'static,
    A: Handle<C, N> + 'static,
    A::State: Clone,
    C: Clone + Debug + Eq + Serialize + DeserializeOwned + Send + Sync + 'static,
    I: Clone + Debug + Ord + Serialize + DeserializeOwned + Send + Sync + 'static,
    EventOf<A>: Clone + Eq + Serialize + DeserializeOwned,
{
    pub(super) fn new(
        store: &Store<S>,
        id: A::Id,
        timeout: Duration,
        pending: Option<Request<C, I>>,
        handoff: Handoff<C, I>,
    ) -> Self {
        Self {
            repository: store.repository().codec(Codec).build(),
            id,
            timeout,
            pending,
            handoff,
        }
    }

    async fn load(&self) -> Result<Root<A, C, I, N>, Error<A::Error, I, S::Error>> {
        trace::step("load", async {
            let root = timeout(self.timeout, self.repository.load(self.id.clone()))
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(Cause::Store)?;
            if root.state().map_err(Cause::Kernel)?.integrity.is_err() {
                return Err(Error::Corrupt);
            }
            Ok(root)
        })
        .await
    }

    fn settle(&mut self, root: &Root<A, C, I, N>) -> Result<(), Error<A::Error, I, S::Error>> {
        let mut stage = self
            .pending
            .as_ref()
            .map(|_| trace::Trace::stage("reconcile"));
        if let Some(pending) = &self.pending
            && root.version() <= pending.expected
        {
            // Empty/stale reads cannot settle a write. A later physical version
            // proves the original CAS committed or can no longer win.
            if let Some(trace) = &mut stage {
                trace.finish("uncertain");
            }
            return Err(Error::Uncertain {
                command: pending.id.clone(),
                source: None,
            });
        }
        self.pending = None;
        if let Some(trace) = &mut stage {
            trace.finish("ok");
        }
        Ok(())
    }

    async fn write(
        &mut self,
        root: &mut Root<A, C, I, N>,
        request: Request<C, I>,
    ) -> Result<(), Error<A::Error, I, S::Error>> {
        let version = root
            .version()
            .map_or(Some(Version::INITIAL), Version::next)
            .ok_or(Cause::Kernel(KernelError::VersionOverflow))?;
        let id = request.id.clone();
        let previous = self.pending.replace(request.clone());
        let decision = Decision { request, version };
        match timeout(self.timeout, self.repository.execute(root, decision)).await {
            Ok(Ok(_)) => Ok(()),
            Err(_) => Err(Error::Uncertain {
                command: id,
                source: None,
            }),
            Ok(Err(ExecuteError::Store(error))) if error.is_conflict() => {
                self.pending = None;
                Err(Error::Conflict)
            }
            Ok(Err(ExecuteError::Decide(error))) => self.failure(previous, error.into()),
            Ok(Err(ExecuteError::Kernel(error))) => self.failure(previous, Cause::Kernel(error)),
            Ok(Err(ExecuteError::Store(error))) => self.store_failure(id, previous, error),
        }
    }

    fn failure(
        &mut self,
        previous: Option<Request<C, I>>,
        error: Cause<A::Error, S::Error>,
    ) -> Result<(), Error<A::Error, I, S::Error>> {
        self.pending = previous;
        match &self.pending {
            // A definite failure of this retry says nothing about the older
            // append. Keep its original identity/base and the new typed cause.
            Some(original) => Err(Error::Uncertain {
                command: original.id.clone(),
                source: Some(Box::new(error)),
            }),
            None => Err(error.into()),
        }
    }

    fn store_failure(
        &mut self,
        id: I,
        previous: Option<Request<C, I>>,
        error: StoreError<S::Error, CodecError, CodecError>,
    ) -> Result<(), Error<A::Error, I, S::Error>> {
        match error {
            StoreError::Encode(_) | StoreError::Envelope(_) | StoreError::VersionOverflow => {
                self.failure(previous, Cause::Store(error))
            }
            // Adapter errors in released Mnesis do not prove COMMIT failed.
            _ => Err(Error::Uncertain {
                command: id,
                source: Some(Box::new(Cause::Store(error))),
            }),
        }
    }

    pub(super) async fn submit(
        &mut self,
        request: Request<C, I>,
    ) -> Result<Receipt<I>, Error<A::Error, I, S::Error>> {
        let mut root = self.load().await?;
        let existing = root
            .state()
            .map_err(Cause::Kernel)?
            .receipt(&request)
            .map_err(|Mismatch| Error::Mismatch)?;
        let can_retry =
            self.pending.as_ref() == Some(&request) && request.expected == root.version();
        let settled = self.settle(&root);
        if let Some(receipt) = existing {
            trace::duplicate();
            return Ok(receipt);
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|original| original.id == request.id && original != &request)
        {
            return Err(Error::Mismatch);
        }
        if let Err(error) = settled
            && !can_retry
        {
            return Err(error);
        }
        if request.expected != root.version() {
            return Err(Error::Conflict);
        }
        match trace::step("append", self.write(&mut root, request.clone())).await {
            Err(Error::Conflict) => {
                let reloaded = self.load().await?;
                reloaded
                    .state()
                    .map_err(Cause::Kernel)?
                    .receipt(&request)
                    .map_err(|Mismatch| Error::Mismatch)?
                    .ok_or(Error::Conflict)
                    .inspect(|_| trace::duplicate())
            }
            Err(error) => Err(error),
            Ok(()) => {
                let observed = root
                    .state()
                    .map_err(Cause::Kernel)
                    .and_then(|state| state.receipt(&request).map_err(|Mismatch| Cause::Corrupt))
                    .and_then(|receipt| receipt.ok_or(Cause::Corrupt));
                match observed {
                    Ok(receipt) => {
                        self.pending = None;
                        trace::accepted();
                        Ok(receipt)
                    }
                    Err(cause) => Err(Error::Uncertain {
                        command: request.id,
                        source: Some(Box::new(cause)),
                    }),
                }
            }
        }
    }

    pub(super) async fn read(
        &mut self,
    ) -> Result<(Option<Version>, A::State), Error<A::Error, I, S::Error>> {
        let root = self.load().await?;
        self.settle(&root)?;
        let state = root.state().map_err(Cause::Kernel)?;
        Ok((root.version(), state.domain.clone()))
    }

    pub(super) async fn original(
        &mut self,
        id: &I,
    ) -> Result<Option<Request<C, I>>, Error<A::Error, I, S::Error>> {
        let root = self.load().await?;
        let original = root.state().map_err(Cause::Kernel)?.original(id);
        let settled = self.settle(&root);
        if original.is_some() {
            return Ok(original);
        }
        settled?;
        Ok(None)
    }

    pub(super) async fn retry(
        &mut self,
        id: &I,
    ) -> Result<Option<Receipt<I>>, Error<A::Error, I, S::Error>> {
        let pending = self
            .pending
            .as_ref()
            .filter(|request| &request.id == id)
            .cloned();
        let original = match pending {
            Some(request) => Some(request),
            None => self.original(id).await?,
        };
        match original {
            Some(request) => self.submit(request).await.map(Some),
            None => Ok(None),
        }
    }
}
