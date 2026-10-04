//! Genie domain core.
//!
//! The task tracker plus the event journal the platform is built on. See
//! `docs/platform/backend.md`.

pub mod automation;
pub mod db;
pub mod error;
pub mod events;
pub mod inbox;
pub mod migrate;
pub mod model;
pub mod repos;
pub mod secrets;
pub mod server_db;
pub mod team;
pub mod tracker;
pub mod usage;
pub mod vault;
pub mod work;

pub use error::{GenieError, Result};
pub use events::Event;
pub use model::*;
pub use tracker::{
    ArtifactContent, ArtifactInput, ArtifactSource, CreateInput, DeletePlan, EpicContext, ListFilter, Meta, StatusOptions, Tracker,
    UpdateInput,
};
