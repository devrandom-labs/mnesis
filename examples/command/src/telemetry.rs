use std::time::Duration;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle, PrometheusRecorder};
use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::error::OTelSdkError;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider, SpanExporter,
};
use thiserror::Error;
use tracing::subscriber::SetGlobalDefaultError;
use tracing_subscriber::prelude::*;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Metrics(#[from] metrics::SetRecorderError<PrometheusRecorder>),
    #[error(transparent)]
    Subscriber(#[from] SetGlobalDefaultError),
    #[error(transparent)]
    Sdk(#[from] OTelSdkError),
}

pub fn install(
    exporter: impl SpanExporter + 'static,
) -> Result<(SdkTracerProvider, PrometheusHandle), Error> {
    let recorder = PrometheusBuilder::new().build_recorder();
    let metrics = recorder.handle();
    metrics::set_global_recorder(recorder)?;
    let batch = BatchSpanProcessor::builder(exporter)
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(128)
                .with_max_export_batch_size(128)
                .with_scheduled_delay(Duration::from_secs(3600))
                .build(),
        )
        .build();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(batch)
        .build();
    let tracer = provider.tracer("mnesis-example-command");
    let subscriber =
        tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
    tracing::subscriber::set_global_default(subscriber)?;
    Ok((provider, metrics))
}
