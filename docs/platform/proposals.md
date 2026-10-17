---
title: Product proposals
summary: Proposed sequencing for project planning, memory, and broader automation.
type: note
status: draft
---

# Product proposals

The [project and task manager lifecycle](project-and-task-managers.md) supersedes the earlier proposal for epic orchestrators: execution now belongs to one manager per task, with shared project context and a separate mandatory retrospective.

These are proposals, not implemented configuration or promised release dates. The [roadmap](roadmap.md) tracks current priorities.

## Phase 0: pilot evidence

Run an isolated pilot on the current orchestration model. Verify output quality, recovery, CI feedback, concurrency limits, budgets, and environment readiness. Use the observed failures as requirements for the next phases.

## Phase 1: project manager and memory

The implemented management lifecycle provides a persistent project manager responsible for planning and owner conversation. Keep durable project memory separate from a growing chat transcript. It should retain goals, constraints, decisions, and current work without reconstructing them from every task.

## Phase 2: epic coordination

The earlier epic-orchestrator boundary is superseded by one manager per task. The project manager controls priorities and releases work into execution. Keep global capacity and budgets explicit so decomposition does not create an unbounded stream of active tasks.

## Phase 3: routines

Build reusable workflows for logs, alerts, issue intake, knowledge cleanup, and morning summaries on the existing automation engine. Add connectors only for a concrete workflow.

## Phase 4: workspaces and richer interaction

Evaluate reproducible project environments, reviewer-directed changes, documentation impact, additional content formats, wider provider integration, and per-person budgets using pilot evidence.

Further design questions include richer memory retrieval, project-wide planning interactions, scheduling overrides, and container-runner isolation guarantees.
