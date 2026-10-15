#!/usr/bin/env bash
# A scripted stand-in for an LLM agent: acts only through `genie`, like a real one.
# The last argument is the turn's message.
set -euo pipefail
msg="${!#}"
g() { "$GENIE_BIN" "$@"; }
echo "turn of $GENIE_AGENT_NAME ($GENIE_AGENT_ROLE)"
if [ -n "${GENIE_JOB:-}" ]; then
  case "$GENIE_AGENT_ROLE" in
    analyst)
      g task artifact --kind analysis --name analysis.md --text "Scope: CSV export of orders."
      g job output '{"questions":[{"text":"Какой формат файла?","why":"от этого зависит библиотека","options":["CSV","XLSX"]},{"text":"Кто получает выгрузку?","why":"права доступа","options":[]}],"draft_acceptance":["файл скачивается","в файле все заказы за период"]}'
      ;;
    documenter)
      g docs write "$GENIE_PROJECT/features/export.md" --text $'---\ntitle: Экспорт заказов\ntype: guide\nstatus: current\n---\n# Экспорт заказов\n\nЗаказы выгружаются в CSV.' --note "export docs"
      g job output '{"pages_changed":["'"$GENIE_PROJECT"'/features/export.md"],"summary":"Описан экспорт заказов.","changelog":{"group":"added","text":"Экспорт заказов в CSV"}}'
      ;;
  esac
  exit 0
fi
case "$GENIE_AGENT_ROLE" in
  orchestrator)
    if grep -q "in the inbox from the owner" <<<"$msg"; then
      id=$(grep -o 'New task [A-Z]*-[0-9]*' <<<"$msg" | head -1 | awk '{print $3}')
      g task update --task "$id" -d "Export orders as CSV" -a "file downloads" --merge-strategy "merge by orchestrator"
      g task status ready --task "$id" --note "refined"
      g team spawn "$id" --template pair --note "small change"
    fi
    if grep -q "verdict" <<<"$msg"; then
      id=$(g task list --status approved | awk 'NR==1{print $1}')
      [ -n "$id" ] && g task status done --task "$id" --note "accepted"
    fi
    ;;
  executor)
    if grep -q "## Kickoff" <<<"$msg"; then
      if [ "${FAIL_ONCE:-}" = "1" ] && [ ! -f "$GENIE_MARK" ]; then touch "$GENIE_MARK"; echo "simulated crash" >&2; exit 3; fi
      g task status in_progress
      g task artifact --kind test-report --name tests.md --text "cargo test: ok"
      g task status review --note "ready for review"
      # Consume all output so `genie team show` cannot write into a pipe whose
      # reader has already exited (which intermittently gives it SIGPIPE).
      reviewer=$(g team show | awk '/\(reviewer\)/ && !found { reviewer=$2; found=1 } END { if (found) print reviewer }')
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
