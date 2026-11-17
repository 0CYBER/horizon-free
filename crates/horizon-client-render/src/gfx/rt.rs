//! Ray tracing (`HORIZON_RAY_TRACING=1`).

/// Hardware ray queries, only on request.
pub(super) mod query;
/// The terrain's acceleration structures (step 2).
pub(super) mod scene;
