#!/usr/bin/env bash
# Re-records the RBAC fixtures (E17-S01) from a kind cluster:
#
#   cluster.json  the Roles, ClusterRoles and bindings the reviews depend on (managedFields,
#                 uid, resourceVersion and timestamps dropped)
#   reviews.json  per subject and namespace: who the API server says the subject is
#                 (`kubectl auth whoami --as`), the SelfSubjectRulesReview status that
#                 `kubectl auth can-i --list --as` reads (as JSON), and the rows it prints
#
# Usage: record.sh [context]   (default kind-oxikube). Applies fixture.yaml, records, deletes it.
# Only objects named oxi-rbac-fx* are created or deleted; nothing shared is touched.
set -euo pipefail

context="${1:-kind-oxikube}"
here="$(cd "$(dirname "$0")" && pwd)"
ns="oxi-rbac-fx"
k() { kubectl --context "$context" "$@"; }

k apply -f "$here/fixture.yaml" >/dev/null
trap 'k delete -f "$here/fixture.yaml" --ignore-not-found --wait=false >/dev/null' EXIT
# The aggregation controller fills the custom aggregated role asynchronously.
for _ in $(seq 1 30); do
  [ "$(k get clusterrole oxi-rbac-fx-aggregate -o json | jq '.rules | length')" -gt 0 ] && break
  sleep 1
done

# subject kind | name | impersonation flags
subjects=(
  "User|fx-viewer|--as=fx-viewer"
  "User|fx-editor|--as=fx-editor"
  "User|fx-admin|--as=fx-admin"
  "User|fx-root|--as=fx-root"
  "Group|fx-team|--as=fx-member --as-group=fx-team"
  "ServiceAccount|$ns/fx-sa|--as=system:serviceaccount:$ns:fx-sa"
)

reviews="[]"
for entry in "${subjects[@]}"; do
  IFS='|' read -r kind name flags <<<"$entry"
  # shellcheck disable=SC2086
  whoami="$(k auth whoami $flags -o json | jq '.status.userInfo | {username, groups}')"
  for namespace in "$ns" default; do
    # shellcheck disable=SC2086
    status="$(printf '{"apiVersion":"authorization.k8s.io/v1","kind":"SelfSubjectRulesReview","spec":{"namespace":"%s"}}' "$namespace" \
      | k create $flags --validate=false -f - -o json | jq '.status')"
    # What `kubectl auth can-i --list` prints for the same review, one row per line, header dropped.
    # shellcheck disable=SC2086
    printed="$(k auth can-i --list $flags -n "$namespace" | tail -n +2 | jq -R . | jq -s .)"
    reviews="$(jq --arg kind "$kind" --arg name "$name" --arg namespace "$namespace" \
      --argjson who "$whoami" --argjson status "$status" --argjson printed "$printed" \
      '. + [{subject: {kind: $kind, name: $name}, user: $who.username, groups: $who.groups, namespace: $namespace, status: $status, can_i_list: $printed}]' \
      <<<"$reviews")"
  done
done
jq '.' <<<"$reviews" >"$here/reviews.json"

# The objects behind the reviews: every binding naming a recorded subject or an implicit group,
# the roles they reference, and every ClusterRole that aggregates into another.
bindings_filter='[.items[] | select(any(.subjects[]?;
    (.kind == "User" and (.name | startswith("fx-")))
    or (.kind == "Group" and (.name == "fx-team" or .name == "system:authenticated"
        or .name == "system:unauthenticated" or (.name | startswith("system:serviceaccounts"))))
    or (.kind == "ServiceAccount" and .namespace == "oxi-rbac-fx")))]'
trim='del(.metadata.managedFields, .metadata.uid, .metadata.resourceVersion,
  .metadata.creationTimestamp, .metadata.annotations["kubectl.kubernetes.io/last-applied-configuration"])
  | if .metadata.annotations == {} then del(.metadata.annotations) else . end'

crbs="$(k get clusterrolebindings -o json | jq "$bindings_filter")"
rbs="$(k get rolebindings -n "$ns" -o json | jq "$bindings_filter")"
roles="$(k get roles -n "$ns" -o json | jq '.items')"
clusterroles="$(k get clusterroles -o json | jq --argjson crbs "$crbs" --argjson rbs "$rbs" '
  ([$crbs[], $rbs[] | select(.roleRef.kind == "ClusterRole") | .roleRef.name]) as $named
  | [.items[] | select((.metadata.name as $n | $named | index($n))
      or any(.metadata.labels // {} | keys[]; startswith("rbac.authorization.k8s.io/aggregate-to-")
        or startswith("oxi-rbac-fx/")))]')"

jq -n --argjson a "$clusterroles" --argjson b "$roles" --argjson c "$crbs" --argjson d "$rbs" \
  "{apiVersion: \"v1\", kind: \"List\", items: ([\$a[], \$b[], \$c[], \$d[]] | map($trim)
    | sort_by(.kind, .metadata.namespace // \"\", .metadata.name))}" >"$here/cluster.json"
echo "recorded $(jq '.items | length' "$here/cluster.json") objects and $(jq 'length' "$here/reviews.json") reviews"
