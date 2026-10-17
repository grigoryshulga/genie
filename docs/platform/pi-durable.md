---
title: Durable runtime experiment
summary: Historical result of the pi-durable prototype; Genie continues to use pi RPC sessions.
type: decision
status: deprecated
search: false
---

# Durable runtime experiment

The October 3, 2026 prototype evaluated `pi-durable` as a replacement for Genie's pi RPC runtime. It used a fake model; real provider authentication, MCP, skills, the Rust bridge, and load were not verified. The prototype is retained under Git tag `archive/proto-durable`.

## Findings

The prototype demonstrated request deduplication, conversation persistence, and interruption recovery. It also required a TypeScript sidecar and a new integration bridge. Its shell ran through bubblewrap, but file tools relied on in-process path checks, giving weaker isolation than placing the entire pi process in the sandbox.

## Decision

Do not migrate on the prototype's evidence. Keep mail, authorization, and supervision in Rust and continue using [pi RPC sessions](agent-bus.md). Re-evaluate a durable library against its current API, full integration tests, and equivalent sandbox guarantees before adopting it.

Useful design ideas remain: record intent before effects, memoize completed effects, and replay tools only when their contract declares that safe. These are not guarantees of the current integration.
