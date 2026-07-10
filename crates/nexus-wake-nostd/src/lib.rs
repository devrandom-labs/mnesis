//! Global-eventcount [`WakeSource`] for `no_std` on-device live-tail
//! subscriptions (#302).
//!
//! # What this is
//!
//! The `no_std` counterpart to `nexus-wake`'s tokio-backed `StreamNotifiers`:
//! a [`WakeSource`] built on `core` atomics plus [`event_listener::Event`]
//! (an eventcount — `no_std` + alloc), so the generic catch-up-then-live-tail
//! loop in `nexus-store` can park on a device with no tokio and no OS.
//!
//! **Optional and executor-dependent.** The primary device model is
//! append-and-sync: events are produced locally and synced to a server,
//! where subscriptions run under tokio via `nexus-wake`. Reach for this
//! crate only when a device genuinely runs an *on-device* live-tail —
//! driving the subscription then also needs a `no_std` executor (e.g.
//! embassy). Any executor works: the only requirement is polling the
//! futures this crate returns.
//!
//! # Global, not per-stream — spurious wakes by design
//!
//! There is ONE eventcount for the whole store: every [`wake`] rouses every
//! parked registration, per-stream and `$all` alike. The
//! [`WakeRegistration::arm`] contract explicitly permits spurious wakes —
//! the subscription loop's response is a single empty re-scan followed by
//! re-parking. Per-stream routing is a throughput optimization for the
//! server-side many-actor-streams workload (that is `StreamNotifiers`' job,
//! with its keyed registry and drop-guard reaping); an on-device live-tail
//! has a handful of subscriptions, so the routing machinery (a map, a
//! `no_std` mutex, guards) would cost more than the false wakes it saves. If
//! a real device workload ever shows herd cost, a routed impl can be added
//! as a second type in this crate — an additive, non-breaking change.
//!
//! # Lost-wakeup discipline
//!
//! [`arm`](WakeRegistration::arm) captures a seen-generation synchronously,
//! then the returned future (1) creates a listener — registered into the
//! notify list AT CREATION, before any check — (2) re-checks the
//! generation, returning if it moved, and (3) otherwise awaits the
//! listener. A wake between `arm` and the first poll bumps the generation
//! (caught by 2); a wake after is delivered to the already-registered
//! listener (caught by 3). No wake after `arm` is ever lost.
//!
//! [`wake`]: WakeSource::wake

#![no_std]

extern crate alloc;
