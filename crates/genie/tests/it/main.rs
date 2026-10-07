//! Integration tests of the server, one binary: each file below was its own test target, and
//! linking ~30 debug binaries dominated the build. Two stay apart, for the state of their whole
//! process: `sandbox.rs` sets `HOME`, `migrate.rs` reads the process's migration reports (which
//! every app opening a tracker drains). Run one file with `cargo test -p genie --test it <file>::`.

mod common;

mod agent_config;
mod api;
mod catalog;
mod channels;
mod console;
mod delivery;
mod docs;
mod engine;
mod git_flow;
mod git_hosts;
mod git_proxy;
mod harness;
mod ideas;
mod litellm_key;
mod mcp_gateway;
mod mcp_server;
mod model_prices;
mod ops;
mod outcome;
mod providers;
mod repos;
mod runtime;
mod scenarios;
mod sessions;
mod tasks;
mod vault_sync;
mod watchdog;
mod worktrees;
