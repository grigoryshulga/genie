#!/usr/bin/env bash
# A stand-in agent that holds its turn for GENIE_SLEEP seconds (default 8) and
# succeeds: for load and slot tests, where the agent must occupy the scheduler.
set -euo pipefail
sleep "${GENIE_SLEEP:-8}"
exit 0
