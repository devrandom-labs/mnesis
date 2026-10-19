//! Read from an already migrated database using a verified server certificate.
use std::env::VarError;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Duration;

use futures::TryStreamExt;
use mnesis_postgres::{PostgresError, PostgresStore};
use mnesis_store::RawEventStore;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use thiserror::Error;

#[derive(Debug, Error)]
enum ExampleError {
    #[error(transparent)]
    Environment(#[from] VarError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Store(#[from] Box<PostgresError>),
    #[error(transparent)]
    Output(#[from] io::Error),
}

async fn read() -> Result<(), ExampleError> {
    let address = std::env::var("DATABASE_URL")?;
    let authority = std::env::var("PGSSLROOTCERT")?;
    let options = address
        .parse::<PgConnectOptions>()?
        .ssl_mode(PgSslMode::VerifyFull)
        .ssl_root_cert(authority);
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?;
    let store = PostgresStore::from_pool(pool.clone())
        .await
        .map_err(Box::new)?;
    let mut events = store.read_all(None).await.map_err(Box::new)?;
    if let Some((_position, _key, event)) = events.try_next().await.map_err(Box::new)? {
        writeln!(io::stdout(), "First event version: {}", event.version())?;
    }
    drop(events);
    drop(store);
    pool.close().await;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    if read().await.is_ok() {
        ExitCode::SUCCESS
    } else {
        // Connection inputs and options can contain secrets, including in Debug.
        let _written = writeln!(
            io::stderr(),
            "Postgres read failed; check configuration and database logs."
        );
        ExitCode::FAILURE
    }
}
