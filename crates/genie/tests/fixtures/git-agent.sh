#!/usr/bin/env bash
# A scripted stand-in for an LLM agent that works with git the way a real one does: plain `git`
# in its working directory (its `origin` is the genie proxy) and `genie pr …`.
# The last argument is the turn's message.
set -euo pipefail
msg="${!#}"
g() { "$GENIE_BIN" "$@"; }
echo "turn of $GENIE_AGENT_NAME ($GENIE_AGENT_ROLE)"
case "$GENIE_AGENT_ROLE" in
  orchestrator)
    if grep -q "in the inbox from the owner" <<<"$msg"; then
      id=$(grep -o 'New task [A-Z]*-[0-9]*' <<<"$msg" | head -1 | awk '{print $3}')
      g task update --task "$id" -d "Export orders as CSV" -a "file downloads" --merge-strategy "a person merges the request"
      g task status ready --task "$id" --note "refined"
      g team spawn "$id" --template pair --note "small change"
    fi
    if grep -q "verdict" <<<"$msg"; then
      id=$(g task list --status approved | awk 'NR==1{print $1}')
      # The request is not merged yet: the server refuses, and the task waits for the merge.
      [ -n "$id" ] && { g task status done --task "$id" --note "accepted" || echo "not closable yet"; }
    fi
    if grep -q "was merged" <<<"$msg"; then
      id=$(g task list --status approved | awk 'NR==1{print $1}')
      [ -n "$id" ] && g task status done --task "$id" --note "merged by a person"
    fi
    ;;
  executor)
    if grep -q "## Kickoff" <<<"$msg"; then
      # The agent reports what it sees as artifacts of its task (the only place a sandboxed agent can write to besides its workspace).
      say() { g task artifact --kind other --name "$1" --text "$2"; }
      # What the agent's environment holds: no token of the git host, no service token for LiteLLM.
      say env.txt "$(env | grep -E 'GENIE_TEST_HOST_TOKEN|GITHUB_TOKEN|GH_TOKEN|LITELLM_INFO_TOKEN' || echo 'no host tokens')"
      say repos.txt "$(g repos list)"
      g task status in_progress
      echo "csv export" > feature.txt
      git add feature.txt
      git commit -q -m "CSV export"
      git push -q origin HEAD
      # Straight to the default branch: refused by the server.
      if refused=$(git push -q origin HEAD:main 2>&1); then echo "PUSHED TO MAIN" >&2; exit 9; fi
      say refused.txt "$refused"
      say pr.txt "$(g pr open --title 'CSV export' --body 'Adds the export.')"
      g task status review --note "request is open"
      # Consume the whole response: early exit can break the CLI's stdout under pipefail.
      reviewer=$(g team show | awk '/\(reviewer\)/ && !found {reviewer=$2; found=1} END {print reviewer}')
      g mail send "$reviewer" "please review" --intent question
    fi
    ;;
  reviewer)
    if grep -q "## Kickoff" <<<"$msg"; then g team set-status "waiting for the executor"; fi
    if grep -q "please review" <<<"$msg"; then
      g task check 1
      g task artifact --kind review --name review.md --text "LGTM"
      g task status approved --note "all criteria met"
      g mail send orchestrator "approved" --intent verdict
    fi
    ;;
esac
