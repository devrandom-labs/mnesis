use std::future::Future;

#[cfg(feature = "tracing")]
use tracing::instrument::WithSubscriber;
#[cfg(feature = "tracing")]
use tracing::{Dispatch, Instrument, Span, debug_span, dispatcher, info, info_span};

use crate::Error;
#[cfg(feature = "metrics")]
use crate::meter::{self, Observation};

#[derive(Clone)]
pub struct Scope {
    #[cfg(feature = "tracing")]
    span: Span,
    #[cfg(feature = "tracing")]
    subscriber: Dispatch,
}
pub struct Trace {
    scope: Scope,
    #[cfg(any(feature = "tracing", feature = "metrics"))]
    outcome: &'static str,
    #[cfg(feature = "metrics")]
    metric: Observation,
}
impl Trace {
    pub fn request(operation: &'static str) -> Self {
        #[cfg(not(feature = "tracing"))]
        let _ = operation;
        Self {
            #[cfg(feature = "metrics")]
            metric: Observation::request(operation),
            scope: Scope::new(|| Scope {
                #[cfg(feature = "tracing")]
                span: info_span!(
                    "mnesis.command.request",
                    operation,
                    outcome = tracing::field::Empty
                ),
                #[cfg(feature = "tracing")]
                subscriber: dispatcher::get_default(Clone::clone),
            }),
            #[cfg(any(feature = "tracing", feature = "metrics"))]
            outcome: "interrupted",
        }
    }
    pub fn worker() -> Self {
        Self {
            #[cfg(feature = "metrics")]
            metric: Observation::Worker,
            scope: Scope::new(|| Scope {
                #[cfg(feature = "tracing")]
                span: info_span!(parent: None, "mnesis.command.worker", outcome = tracing::field::Empty),
                #[cfg(feature = "tracing")]
                subscriber: dispatcher::get_default(Clone::clone),
            }),
            #[cfg(any(feature = "tracing", feature = "metrics"))]
            outcome: "interrupted",
        }
    }
    pub fn stage(stage: &'static str) -> Self {
        #[cfg(not(feature = "tracing"))]
        let _ = stage;
        Self {
            #[cfg(feature = "metrics")]
            metric: Observation::stage(stage),
            scope: Scope::new(|| Scope {
                #[cfg(feature = "tracing")]
                span: debug_span!(
                    "mnesis.command.stage",
                    stage,
                    outcome = tracing::field::Empty
                ),
                #[cfg(feature = "tracing")]
                subscriber: dispatcher::get_default(Clone::clone),
            }),
            #[cfg(any(feature = "tracing", feature = "metrics"))]
            outcome: "interrupted",
        }
    }
    pub fn scope(&self) -> Scope {
        self.scope.clone()
    }
}
impl Trace {
    pub fn child(&self, stage: &'static str) -> Self {
        self.scope().sync(|| Self::stage(stage))
    }
    pub const fn finish(&mut self, outcome: &'static str) {
        #[cfg(any(feature = "tracing", feature = "metrics"))]
        {
            self.outcome = outcome;
        }
        #[cfg(not(any(feature = "tracing", feature = "metrics")))]
        let _ = (self, outcome);
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        #[cfg(feature = "tracing")]
        self.scope.span.record("outcome", self.outcome);
        #[cfg(feature = "metrics")]
        self.metric.finish(self.outcome);
    }
}
impl Scope {
    fn new(factory: impl FnOnce() -> Self) -> Self {
        #[cfg(feature = "tracing")]
        {
            factory()
        }
        #[cfg(not(feature = "tracing"))]
        {
            drop(factory);
            Self {}
        }
    }
    pub fn sync<T>(&self, body: impl FnOnce() -> T) -> T {
        #[cfg(feature = "tracing")]
        {
            dispatcher::with_default(&self.subscriber, || self.span.in_scope(body))
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = self;
            body()
        }
    }
    pub async fn run<F: Future>(self, future: F) -> F::Output {
        #[cfg(feature = "tracing")]
        {
            future
                .instrument(self.span.clone())
                .with_subscriber(self.subscriber.clone())
                .await
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = self;
            future.await
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        #[cfg(feature = "tracing")]
        dispatcher::with_default(&self.subscriber, || {
            let span = std::mem::replace(&mut self.span, Span::none());
            drop(span);
        });
    }
}
pub async fn step<F, T, D, I, E>(stage: &'static str, future: F) -> Result<T, Error<D, I, E>>
where
    F: Future<Output = Result<T, Error<D, I, E>>>,
{
    let mut trace = Trace::stage(stage);
    trace
        .scope()
        .run(async move {
            let result = future.await;
            trace.finish(outcome(&result));
            result
        })
        .await
}
pub fn sync<T, E>(stage: &'static str, body: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    let mut trace = Trace::stage(stage);
    trace.scope().sync(|| {
        let result = body();
        trace.finish(if result.is_ok() { "ok" } else { "failed" });
        result
    })
}
pub fn invalid_request() {
    let mut trace = Trace::request("submit");
    let mut admission = trace.child("admission");
    trace.finish("request");
    admission.finish("request");
}
pub const fn outcome<T, D, I, E>(result: &Result<T, Error<D, I, E>>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => match error {
            Error::Failed(_) => "failed",
            Error::Mismatch => "mismatch",
            Error::Conflict => "conflict",
            Error::QueueFull => "queue_full",
            Error::QueueLimit { .. } => "queue_limit",
            Error::DeadlineZero => "deadline_zero",
            Error::Closed => "closed",
            Error::Scope => "scope",
            Error::Runtime(_) => "runtime",
            Error::Uncertain { .. } => "uncertain",
            Error::Request(_) => "request",
            Error::Corrupt => "corrupt",
            Error::Timeout => "timeout",
        },
    }
}
#[cfg(any(feature = "tracing", feature = "metrics"))]
pub fn accepted() {
    #[cfg(feature = "metrics")]
    meter::count("mnesis_command_acceptances_total");
    #[cfg(feature = "tracing")]
    info!(name: "mnesis.command.accepted", outcome = "ok");
}
#[cfg(any(feature = "tracing", feature = "metrics"))]
pub fn duplicate() {
    #[cfg(feature = "metrics")]
    meter::count("mnesis_command_duplicates_total");
    #[cfg(feature = "tracing")]
    info!(name: "mnesis.command.duplicate", outcome = "ok");
}
#[cfg(feature = "tracing")]
pub fn reply_dropped() {
    info!(name: "mnesis.command.reply_dropped", outcome = "cancelled");
}
#[cfg(not(any(feature = "tracing", feature = "metrics")))]
pub const fn accepted() {}
#[cfg(not(any(feature = "tracing", feature = "metrics")))]
pub const fn duplicate() {}
#[cfg(not(feature = "tracing"))]
pub const fn reply_dropped() {}
