//! Workspace environments on the server: the paths a workspace's processes see and the runner
//! contract that builds and runs them.
//!
//! Runners (`podman`, `host`) implement the `runner` contract; the `store` keeps snapshots and
//! views, `inputs` keys them, the `catalog` holds recipes and profiles, `harness` lets agents run
//! inside a workspace, and the `controller` brings workspaces up, stops, deletes and collects them.

pub mod catalog;
pub mod controller;
pub mod harness;
pub mod host;
pub mod inputs;
pub mod layout;
pub mod podman;
pub mod runner;
pub mod store;
