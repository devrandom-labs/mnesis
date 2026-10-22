use std::time::Instant;

use metrics::{Gauge, Histogram, counter, gauge, histogram};
use tokio::task::JoinError;

pub enum Observation {
    Request(&'static str),
    Stage {
        name: &'static str,
        started: Instant,
    },
    Worker,
}
impl Observation {
    pub fn request(operation: &'static str) -> Self {
        counter!("mnesis_command_submitted_total", "operation" => operation).increment(1);
        Self::Request(operation)
    }
    pub fn stage(name: &'static str) -> Self {
        Self::Stage {
            name,
            started: Instant::now(),
        }
    }
    pub fn finish(&self, outcome: &'static str) {
        match self {
            Self::Request(operation) => match outcome {
                "ok" | "interrupted" => (),
                "uncertain" => unknown(operation),
                reason => {
                    counter!("mnesis_command_rejected_total", "operation" => *operation, "outcome" => reason).increment(1);
                    if reason == "conflict" {
                        count("mnesis_command_conflicts_total");
                    }
                }
            },
            Self::Stage { name, started } => {
                if *name != "queue" {
                    histogram!("mnesis_command_stage_seconds", "stage" => *name, "outcome" => outcome).record(started.elapsed().as_secs_f64());
                }
                if *name == "reconcile" {
                    counter!("mnesis_command_reconciliations_total", "outcome" => outcome)
                        .increment(1);
                }
            }
            Self::Worker => (),
        }
    }
}
pub fn count(name: &'static str) {
    counter!(name).increment(1);
}
pub fn admitted(operation: &'static str) {
    counter!("mnesis_command_admitted_total", "operation" => operation).increment(1);
}
pub fn unknown(operation: &'static str) {
    counter!("mnesis_command_uncertain_total", "operation" => operation).increment(1);
}
pub fn failure(error: &JoinError) {
    let cause = if error.is_cancelled() {
        "cancelled"
    } else {
        "panic"
    };
    counter!("mnesis_command_worker_failures_total", "cause" => cause).increment(1);
}
pub fn shutdown(started: Instant) {
    histogram!("mnesis_command_shutdown_seconds").record(started.elapsed().as_secs_f64());
}
pub struct Slot {
    gauge: Gauge,
    timer: Option<(Instant, Histogram)>,
}
impl Slot {
    fn new(gauge: Gauge, timer: Option<Histogram>) -> Self {
        gauge.increment(1.0);
        Self {
            gauge,
            timer: timer.map(|histogram| (Instant::now(), histogram)),
        }
    }
    pub fn reserved() -> Self {
        Self::new(gauge!("mnesis_command_reserved"), None)
    }
    pub fn queued() -> Self {
        Self::new(
            gauge!("mnesis_command_queued"),
            Some(histogram!("mnesis_command_queue_seconds")),
        )
    }
    pub fn active(operation: &'static str) -> Self {
        Self::new(
            gauge!("mnesis_command_inflight"),
            Some(histogram!("mnesis_command_execution_seconds", "operation" => operation)),
        )
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        if let Some((started, histogram)) = &self.timer {
            histogram.record(started.elapsed().as_secs_f64());
        }
        self.gauge.decrement(1.0);
    }
}
