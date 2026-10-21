use std::fmt::Debug;
use std::time::Duration;

use mnesis::{AggregateRoot, EventOf, Handle, Version};
use mnesis_store::{
    CommandRepository, EventStore, ExecuteError, RawEventStore, Repository, Store, StoreError,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use crate::model::{Adapted, Codec, Decision};
use crate::{Error, Receipt, Request};

type Root<A, C, I, const N: usize> = AggregateRoot<Adapted<A, C, I, N>>;

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
    ) -> Self {
        Self {
            repository: store.repository().codec(Codec).build(),
            id,
            timeout,
            pending,
        }
    }

    pub(super) fn unresolved(self) -> (A::Id, Option<Request<C, I>>) {
        (self.id, self.pending)
    }

    async fn load(&self) -> Result<Root<A, C, I, N>, Error<A::Error, I>> {
        let root = timeout(self.timeout, self.repository.load(self.id.clone()))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|error| Error::Store(Box::new(error)))?;
        if root.state().map_err(|_| Error::Corrupt)?.integrity.is_err() {
            return Err(Error::Corrupt);
        }
        Ok(root)
    }

    fn settle(&mut self, root: &Root<A, C, I, N>) -> Result<(), Error<A::Error, I>> {
        if let Some(pending) = &self.pending
            && root.version() <= pending.expected
        {
            // Empty/stale reads cannot settle a write. A later physical version
            // proves the original CAS committed or can no longer win.
            return Err(Error::Uncertain {
                command: pending.id.clone(),
                source: None,
            });
        }
        self.pending = None;
        Ok(())
    }

    async fn write(
        &mut self,
        root: &mut Root<A, C, I, N>,
        request: Request<C, I>,
    ) -> Result<(), Error<A::Error, I>> {
        let version = root
            .version()
            .map_or(Some(Version::INITIAL), Version::next)
            .ok_or(Error::Capacity)?;
        let id = request.id.clone();
        let previous = self.pending.replace(request.clone());
        let decision = Decision { request, version };
        match timeout(self.timeout, self.repository.execute(root, decision)).await {
            Ok(Ok(_)) => {
                self.pending = None;
                Ok(())
            }
            Err(_) => Err(Error::Uncertain {
                command: id,
                source: None,
            }),
            Ok(Err(ExecuteError::Store(error))) if error.is_conflict() => {
                self.pending = None;
                Err(Error::Conflict)
            }
            Ok(Err(ExecuteError::Decide(error))) => self.failure(previous, error),
            Ok(Err(ExecuteError::Kernel(error))) => {
                self.failure(previous, Error::Store(Box::new(error)))
            }
            Ok(Err(ExecuteError::Store(error))) => self.store_failure(id, previous, error),
        }
    }

    fn failure(
        &mut self,
        previous: Option<Request<C, I>>,
        error: Error<A::Error, I>,
    ) -> Result<(), Error<A::Error, I>> {
        self.pending = previous;
        match &self.pending {
            // A definite failure of this retry says nothing about the older
            // append. Keep its original identity/base and the new typed cause.
            Some(original) => Err(Error::Uncertain {
                command: original.id.clone(),
                source: Some(Box::new(error)),
            }),
            None => Err(error),
        }
    }

    fn store_failure(
        &mut self,
        id: I,
        previous: Option<Request<C, I>>,
        error: StoreError<S::Error, serde_json::Error, serde_json::Error>,
    ) -> Result<(), Error<A::Error, I>> {
        match error {
            StoreError::Encode(_) | StoreError::Envelope(_) | StoreError::VersionOverflow => {
                self.failure(previous, Error::Store(Box::new(error)))
            }
            // Adapter errors in released Mnesis do not prove COMMIT failed.
            _ => Err(Error::Uncertain {
                command: id,
                source: Some(Box::new(error)),
            }),
        }
    }

    pub(super) async fn submit(
        &mut self,
        request: Request<C, I>,
    ) -> Result<Receipt<I>, Error<A::Error, I>> {
        let mut root = self.load().await?;
        let existing = root
            .state()
            .map_err(|_| Error::Corrupt)?
            .receipt(&request)?;
        let can_retry =
            self.pending.as_ref() == Some(&request) && request.expected == root.version();
        let settled = self.settle(&root);
        if let Some(receipt) = existing {
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
        match self.write(&mut root, request.clone()).await {
            Err(Error::Conflict) => {
                let reloaded = self.load().await?;
                reloaded
                    .state()
                    .map_err(|_| Error::Corrupt)?
                    .receipt(&request)?
                    .ok_or(Error::Conflict)
            }
            Err(error) => Err(error),
            Ok(()) => root
                .state()
                .map_err(|_| Error::Corrupt)?
                .receipt(&request)?
                .ok_or(Error::Corrupt),
        }
    }

    pub(super) async fn read(&mut self) -> Result<(Option<Version>, A::State), Error<A::Error, I>> {
        let root = self.load().await?;
        self.settle(&root)?;
        let state = root.state().map_err(|_| Error::Corrupt)?;
        Ok((root.version(), state.domain.clone()))
    }

    pub(super) async fn original(
        &mut self,
        id: &I,
    ) -> Result<Option<Request<C, I>>, Error<A::Error, I>> {
        let root = self.load().await?;
        let original = root.state().map_err(|_| Error::Corrupt)?.original(id);
        let settled = self.settle(&root);
        if original.is_some() {
            return Ok(original);
        }
        settled?;
        Ok(None)
    }

    pub(super) async fn retry(&mut self, id: &I) -> Result<Option<Receipt<I>>, Error<A::Error, I>> {
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
