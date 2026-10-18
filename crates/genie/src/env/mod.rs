//! Workspace environments on the server: the paths a workspace's processes see and the runner
//! contract that builds and runs them.
//!
//! This module is being filled in phases. Runners (`podman`, `host`), the store, inputs, the
//! catalog and the controller come in later modules; they build on `layout` and `runner` as they
//! are here.

pub mod host;
pub mod inputs;
pub mod layout;
pub mod runner;
pub mod store;
