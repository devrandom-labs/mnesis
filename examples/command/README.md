# Command processor

Run from the repository's Nix environment:

```sh
cargo run -p mnesis-example-command --locked
cargo run -p mnesis-example-command --locked --features telemetry
```

The application supplies Tokio, a register aggregate and an in-memory store.
It uses Tower readiness and call, repeats the exact request, accepts a no-op,
reads state, stops and observes the worker, resumes and finds the original
receipt. The result is two acceptance records, one fact and state `7→7`.
In-memory data lasts only for this process; use a persistent adapter for recovery
across process loss.

Readiness reserves bounded queue capacity; it is not database health or command
acceptance. Once admitted, dropping a response or an outer Tower timeout does
not prove rollback. Retain the original ID, expected version and command until
settled. Retry the exact request; never invent a new ID after an uncertain write.
The example retains originals, typed errors and the observed shutdown report
on failure. Applications must also retain originals across abrupt process loss.

The optional `telemetry` feature installs application-owned global collectors:
`tracing-opentelemetry` exports spans through the OpenTelemetry SDK's standard
bounded batch processor to its stdout exporter; `metrics-exporter-prometheus`
collects the metric facade and renders Prometheus text without an HTTP server.
An application root span becomes the parent of request spans across the queue.
The default executable installs neither collector. The processor library never
installs collectors, starts exporters or configures an OpenTelemetry provider.

The application stops and observes both command workers before shutting down
the SDK. SDK flush/shutdown are blocking operations. Its standard batch processor
does not impose a timeout on an arbitrary export callback: a production exporter
must enforce its own finite transport deadline. Stdout here is a demonstration,
not a network collector with such a deadline. A blocking subscriber/recorder can
block the caller or worker; a blocking exporter can stall SDK completion.
Bounded SDK queues can drop spans; metrics and traces are observational and do
not replace durable acceptance records or prove rollback.

The telemetry test uses the real SDK, native in-memory exporter and Prometheus
recorder. It checks exported parent context, exact counters and released gauges,
then makes the exporter return a concrete failure before repeating the same
successful command flow. It observes failed flushes and SDK shutdown. This
checks exporter-interface unavailability, not a network outage or queue overflow.
An integration test also executes the actual binary in each feature mode.
SDK test support is a dev-dependency; normal telemetry builds enable trace only.

See the [processor contracts](../../crates/command/README.md) for exact metric
names, units, label vocabulary, worker custody and supported middleware ordering.
