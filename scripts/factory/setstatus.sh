#!/usr/bin/env bash
# Move a story on the Oxikube project board (project 2) and stamp its Team field.
# usage: scripts/factory/setstatus.sh <issue-number> <Backlog|Ready|In Progress|In Review|Done> [team]
# Used by the oxikube-epics workflow (.claude/workflows/oxikube-epics.js); safe to run by hand.
set -euo pipefail
n="$1"; s="$2"; team="${3:-ultracode}"
case "$s" in
  Backlog) opt=96d681ac ;;
  Ready) opt=3d8d8808 ;;
  "In Progress") opt=24b9a16f ;;
  "In Review") opt=19a72821 ;;
  Done) opt=45edce20 ;;
  *) echo "unknown status: $s" >&2; exit 2 ;;
esac
project=PVT_kwHOAx7J3M4Blha5
item=$(gh api graphql -f query='query($n:Int!){repository(owner:"karan-vk",name:"Oxikube"){issue(number:$n){projectItems(first:5){nodes{id project{number}}}}}}' -F n="$n" -q '.data.repository.issue.projectItems.nodes[]|select(.project.number==2)|.id')
if [ -z "$item" ]; then echo "#$n is not on project 2" >&2; exit 1; fi
gh project item-edit --project-id "$project" --id "$item" --field-id PVTSSF_lAHOAx7J3M4Blha5zhkN_j8 --single-select-option-id "$opt" >/dev/null
gh project item-edit --project-id "$project" --id "$item" --field-id PVTF_lAHOAx7J3M4Blha5zhkN_wk --text "$team" >/dev/null
echo "#$n -> $s ($team)"
