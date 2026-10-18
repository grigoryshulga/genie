//! Observation plane in the server: the application service, the scheduler and the source adapters
//! (`docs/platform/observation-plane.md`). The domain types live in `genie_core::observation`.

pub mod adapters;
pub mod scheduler;
pub mod service;

pub use service::arm_problems;
