mod domain;
#[cfg(feature = "telemetry")]
mod telemetry;
#[cfg(test)]
mod tests;

use std::convert::Infallible;
use std::io::{self, Write};
use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;

use mnesis::Version;
use mnesis_command::{CommandProcessor, Limits, Receipt, Request, Shutdown};
use mnesis_inmemory::InMemoryStore;
use mnesis_store::{RawEventStore, Store};
use thiserror::Error;
use tower::{Service, ServiceExt};
#[cfg(feature = "telemetry")]
use tracing::Instrument;

use domain::Register;

type CommandError =
    mnesis_command::Error<Infallible, NonZeroU64, <InMemoryStore as RawEventStore>::Error>;

#[derive(Debug, Error)]
enum Error {
    #[error("processor startup failed")]
    Startup {
        #[source]
        cause: CommandError,
        previous: Option<Box<Shutdown<u64>>>,
        originals: [Request<u64>; 2],
    },
    #[error("processor stopped before a known successful outcome")]
    Stopped {
        #[source]
        cause: Option<CommandError>,
        report: Box<Shutdown<u64>>,
        originals: [Request<u64>; 2],
    },
    #[error(transparent)]
    Output(#[from] io::Error),
    #[cfg(feature = "telemetry")]
    #[error(transparent)]
    Telemetry(#[from] telemetry::Error),
}
#[derive(Debug)]
struct Summary {
    first: Receipt,
    duplicate: Receipt,
    noop: Receipt,
    repeated: Option<Receipt>,
    before: u64,
    after: u64,
}

const fn originals() -> [Request<u64>; 2] {
    [
        Request {
            id: NonZeroU64::MIN,
            expected: None,
            command: 7,
        },
        Request {
            id: NonZeroU64::MAX,
            expected: Some(Version::INITIAL),
            command: 7,
        },
    ]
}
const fn limits() -> Limits {
    Limits {
        queue: NonZeroUsize::MIN,
        operation: Duration::from_secs(1),
    }
}
fn complete<T>(
    outcome: Result<T, CommandError>,
    report: Shutdown<u64>,
    originals: [Request<u64>; 2],
) -> Result<(T, Shutdown<u64>), Error> {
    match outcome {
        Ok(value) if report.cause().is_none() && report.uncertain().is_none() => {
            Ok((value, report))
        }
        result => Err(Error::Stopped {
            cause: result.err(),
            report: Box::new(report),
            originals,
        }),
    }
}
async fn run(store: &Store<InMemoryStore>) -> Result<Summary, Error> {
    let originals = originals();
    let (mut host, worker) =
        CommandProcessor::<Register, u64>::start(store, "example".into(), limits()).map_err(
            |cause| Error::Startup {
                cause,
                previous: None,
                originals: originals.clone(),
            },
        )?;
    let result = async {
        let first = host.ready().await?.call(originals[0].clone()).await?;
        let duplicate = host.submit(originals[0].clone()).await?;
        let noop = host.submit(originals[1].clone()).await?;
        let before = host.read().await?.1.0;
        Ok((first, duplicate, noop, before))
    }
    .await;
    host.stop();
    let ((first, duplicate, noop, before), report) =
        complete(result, worker.await, originals.clone())?;
    let (resumed, resumed_worker) =
        CommandProcessor::<Register, u64>::resume(store, "example".into(), limits(), &report)
            .map_err(|cause| Error::Startup {
                cause,
                previous: Some(Box::new(report)),
                originals: originals.clone(),
            })?;
    let recovery = async {
        let repeated = resumed.retry(originals[0].id).await?;
        let after = resumed.read().await?.1.0;
        Ok((repeated, after))
    }
    .await;
    resumed.stop();
    let ((repeated, after), _) = complete(recovery, resumed_worker.await, originals)?;
    Ok(Summary {
        first,
        duplicate,
        noop,
        repeated,
        before,
        after,
    })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let store = Store::new(InMemoryStore::new());
    #[cfg(feature = "telemetry")]
    let (provider, metrics) = telemetry::install(opentelemetry_stdout::SpanExporter::default())?;
    #[cfg(feature = "telemetry")]
    let result = run(&store)
        .instrument(tracing::info_span!("application"))
        .await;
    #[cfg(not(feature = "telemetry"))]
    let result = run(&store).await;
    #[cfg(feature = "telemetry")]
    let flushed = provider.shutdown().map_err(telemetry::Error::Sdk);
    let summary = result?;
    #[cfg(feature = "telemetry")]
    flushed?;
    let mut output = io::stdout().lock();
    writeln!(
        output,
        "first={:?} duplicate={:?} noop={:?} retry={:?} state={}→{}",
        summary.first,
        summary.duplicate,
        summary.noop,
        summary.repeated,
        summary.before,
        summary.after
    )?;
    #[cfg(feature = "telemetry")]
    writeln!(output, "{}", metrics.render())?;
    Ok(())
}
