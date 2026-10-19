//! Workspace environments: the domain of a task's prepared environment.
//!
//! `spec` reads and resolves profiles and recipes, `caps` decides whether a runner can satisfy
//! them, `key` gives snapshots their identity, `lock` records what a workspace was built from,
//! and `db` persists workspaces, stages, snapshots, pins and secrets in `server.db`. Running
//! anything is the server crate's job (`genie::env`).

pub mod caps;
pub mod db;
pub mod key;
pub mod lock;
pub mod spec;
