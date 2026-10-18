//! Workspace environments on the server: the paths a workspace's processes see and the runner
//! contract that builds and runs them.
//!
//! This module is being filled in phases. The host runner, the store, inputs, the catalog and the
//! controller come in later modules; they build on `layout`, `runner` and `podman` as they are here.

pub mod catalog;
pub mod host;
pub mod inputs;
pub mod layout;
pub mod podman;
pub mod runner;
pub mod store;
