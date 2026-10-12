"""Read-only GitHub collection and reproducible delivery metrics (Python stdlib)."""

import collections
import datetime as dt
import json
import math
import subprocess
import urllib.parse

UTC = dt.timezone.utc


def timestamp(value):
    parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        raise ValueError("timestamps must include a timezone, for example 2026-10-03T00:00:00Z")
    return parsed.astimezone(UTC)


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", path], text=True))


def collect(repo, since, until):
    """Paginate all PRs; shard runs by day to avoid GitHub's 1000 cap."""
    pulls = []
    page = 1
    while True:
        batch = api(f"repos/{repo}/pulls?state=all&sort=updated&direction=desc&per_page=100&page={page}")
        pulls.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    runs = {}
    day = since.replace(hour=0, minute=0, second=0, microsecond=0)
    while day < until:
        end = day + dt.timedelta(days=1)
        query = urllib.parse.urlencode({"created": f"{day:%Y-%m-%d}"})
        page = 1
        while True:
            result = api(f"repos/{repo}/actions/runs?{query}&per_page=100&page={page}")
            if result["total_count"] >= 1000:
                raise ValueError(f"{day:%Y-%m-%d}: >=1000 runs; split into smaller intervals before reporting")
            batch = result["workflow_runs"]
            for run in batch:
                if since <= timestamp(run["created_at"]) < until:
                    runs[run["id"]] = run
            if len(batch) < 100:
                break
            page += 1
        day = end
    # Keep only operational metadata, never PR bodies, patches, logs or credentials.
    return {
        "schema": 1, "repo": repo, "since": since.isoformat(), "until": until.isoformat(),
        "collected_at": dt.datetime.now(UTC).isoformat(),
        "pulls": [{k: p[k] for k in ("number", "title", "state", "draft", "created_at", "updated_at", "merged_at", "closed_at", "html_url")} for p in pulls],
        "runs": [{k: r[k] for k in ("id", "name", "path", "event", "head_sha", "head_branch", "run_attempt", "status", "conclusion", "created_at", "updated_at", "html_url")} for r in runs.values()],
    }


def percentile(values, fraction):
    """Nearest-rank percentile; null when the cohort is empty."""
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)] if values else None


def summary(snapshot):
    since, until = timestamp(snapshot["since"]), timestamp(snapshot["until"])
    if snapshot["schema"] != 1 or until <= since:
        raise ValueError("unsupported snapshot schema or invalid interval")
    merged = [p for p in snapshot["pulls"] if p["merged_at"] and since <= timestamp(p["merged_at"]) < until]
    hours = [(timestamp(p["merged_at"]) - timestamp(p["created_at"])).total_seconds() / 3600 for p in merged]
    days = (until - since).total_seconds() / 86400
    # PR state is observed at collection, not reconstructed at an earlier cutoff.
    opened = [p for p in snapshot["pulls"] if p["state"] == "open"]
    workflows = {}
    groups = collections.defaultdict(list)
    for run in snapshot["runs"]:
        if since <= timestamp(run["created_at"]) < until:
            groups[(run["path"], run["event"])].append(run)
    for (path, event), runs in sorted(groups.items()):
        completed = [r for r in runs if r["status"] == "completed"]
        elapsed = [(timestamp(r["updated_at"]) - timestamp(r["created_at"])).total_seconds() / 60 for r in completed]
        workflows[f"{path} ({event})"] = {
            "runs": len(runs), "conclusions": dict(collections.Counter(r["conclusion"] or "pending" for r in runs)),
            "elapsed_minutes_p50": percentile(elapsed, .5), "elapsed_minutes_p90": percentile(elapsed, .9),
            "additional_attempts": sum(max(0, r["run_attempt"] - 1) for r in runs),
        }
    return {
        "repo": snapshot["repo"], "since": snapshot["since"], "until": snapshot["until"],
        "merged_prs": len(merged), "merged_per_24h": len(merged) / days,
        "pr_open_to_merge_hours_p50": percentile(hours, .5), "pr_open_to_merge_hours_p90": percentile(hours, .9),
        "merged_by_utc_day": dict(sorted(collections.Counter(p["merged_at"][:10] for p in merged).items())),
        "open_prs_at_collection": len(opened),
        "oldest_open_prs": [{"number": p["number"], "created_at": p["created_at"], "draft": p["draft"]} for p in sorted(opened, key=lambda p: p["created_at"])[:10]],
        "workflows": workflows,
        "limitations": [
            "PR-open-to-merge excludes implementation before PR creation; it is not story lead time.",
            "Merged cohorts exclude unfinished PRs. Open PRs are reported separately at collection time.",
            "Workflow elapsed time includes queueing and uses updated_at; it is not billable runner time.",
            "Only the latest conclusion per run ID is available; additional attempts do not reveal first-pass yield.",
            "Cancellations may be intentional supersession; failures do not by themselves establish flaky tests.",
            "No agent active time, token cost, deployment frequency or production defect rate is measured.",
        ],
    }
