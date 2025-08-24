//! The NixOps4 state is a set of JSON patches that results in a state according
//! to the state schema.
//!
//! This creates a small degree of coupling between state providers and the
//! nixops4 core, which we accept, as
//! 1. It allows state providers to snapshot the final state
//! 2. The amount of coupling is minimal, as we rely on JSON Patch to do most of
//!    the heavy lifting.
//!    For instance, the core can add new fields to the state schema, and the
//!    state providers will not need to be updated.
//! 3. Changing the state schema is expensive anyway, because it is about
//!    persisted data.
//!
//! ## JSON Patch Integration
//!
//! This module uses the `json-patch` crate to handle incremental state
//! updates via RFC 6902 JSON Patch documents. This approach provides:
//!
//! - **Incremental updates**: Only changes are stored, not full state snapshots
//! - **Auditability**: Each change is recorded as a separate patch event
//! - **Conflict resolution**: Patches can be analyzed for conflicts
//! - **Interoperability**: JSON Patch is a standard format

pub mod schema;
