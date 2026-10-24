---
title: Project environments
summary: Provision the tools agents need and understand current sandbox limitations.
type: guide
status: current
---

# Project environments

Agents use the toolchain installed on the Genie host or in its Docker image. Genie does not yet provision a reproducible environment from a project manifest.

## Prepare a project

Before the pilot, identify:

1. Runtime and package-manager versions used in CI.
2. System libraries and services required for builds and tests.
3. Package registries and their authentication requirements.
4. Dependency installation, build, and test commands.
5. Cache locations and expected disk, memory, and runtime costs.

Install those tools as the operator, then verify the build under the service user's sandbox. Put project-specific instructions in the repository so agents can find the supported commands.

## Host deployment

Bubblewrap mounts the system read-only and permits writes in the team's workspace and selected tool caches such as npm, Cargo, Gradle, Maven, and Bun caches. The network is shared.

Agents can install project dependencies in the worktree and permitted caches. They cannot install system packages. A tool manager shim can run an installed runtime, but downloading a new toolchain fails when its installation directory is outside writable mounts.

## Docker deployment

Add required Debian packages with `EXTRA_APT_PACKAGES`; include Rust with `RUST_TOOLCHAIN`. Rebuild the image before launching teams. See [Docker deployment](docker.md#project-tools-and-resource-limits).

A server container is not an environment per project. Its agents share the image's tool versions. Keep secrets and privileged service sockets outside agent access.

## Proposed provisioning

The research in G-90 considered mise manifests, devcontainers, project Dockerfiles, and Nix. The proposed first step is a sandboxed setup phase with pinned tools and a cache keyed by dependency manifests. Projects needing services or system packages would later use a rootless container runner.

This is planned work, not an available `environment` configuration API. Do not rely on automatic tool installation today. The [workspace environments design](environment-design.md) proposes the model: profiles and recipes, runners, prepared-dependency snapshots, isolation policy, lifecycle, and multi-repository checks.
