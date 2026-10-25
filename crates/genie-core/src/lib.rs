//! Genie domain core.
//!
//! The task tracker plus the event journal the platform is built on. See
//! `docs/platform/backend.md`.

pub mod automation;
pub mod connections;
pub mod db;
pub mod decisions;
pub mod env;
pub mod error;
pub mod events;
pub mod inbox;
pub mod intelligence;
pub mod management;
pub mod migrate;
pub mod model;
pub mod observation;
pub mod repos;
pub mod secrets;
pub mod server_db;
pub mod social_auth;
pub mod team;
pub mod telegram;
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
