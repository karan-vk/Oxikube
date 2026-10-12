#!/usr/bin/env python3
"""Portable task packets, local worktree claims, gate receipts and GitHub metrics."""

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
from subprocess import run as run_command
import sys
import tempfile
import time

import metrics


def git(*args, cwd=None):
    return subprocess.check_output(["git", *args], cwd=cwd, text=True).strip()


def root():
    return Path(git("rev-parse", "--show-toplevel"))


def now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=path.parent, delete=False) as stream:
        json.dump(data, stream, indent=2)
        stream.write("\n")
        temporary = stream.name
    os.replace(temporary, path)


def fingerprint(directory):
    """Bind evidence to HEAD, tracked changes and non-ignored untracked contents."""
    digest = hashlib.sha256(git("rev-parse", "HEAD", cwd=directory).encode())
    digest.update(subprocess.check_output(["git", "diff", "HEAD", "--binary"], cwd=directory))
    paths = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=directory)
    for name in sorted(p for p in paths.split(b"\0") if p):
        digest.update(name + b"\0")
        path = directory / os.fsdecode(name)
        digest.update(str(path.lstat().st_mode).encode() + b"\0")
        digest.update(os.readlink(path).encode() if path.is_symlink() else path.read_bytes())
    return digest.hexdigest()


def claims_dir():
    return Path(git("rev-parse", "--path-format=absolute", "--git-common-dir")) / "factory" / "claims"


def start(args):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", args.task):
        raise ValueError("task must contain only letters, digits and hyphens")
    if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", args.slug):
        raise ValueError("slug must be lowercase kebab-case")
    directory = Path(args.worktree).resolve()
    branch = f"story/{args.task}-{args.slug}"
    base = git("rev-parse", "--verify", f"{args.base}^{{commit}}")
    claim = claims_dir() / f"{args.task}.json"
    claim.parent.mkdir(parents=True, exist_ok=True)
    packet = {"schema": 1, "task": args.task, "issue": args.issue,
              "objective": args.objective, "branch": branch, "base_sha": base,
              "worktree": str(directory), "owner": args.owner, "phase": "implement",
              "created_at": now()}
    # O_EXCL makes duplicate claims fail across harnesses sharing this clone.
    with claim.open("x") as stream:
        json.dump(packet, stream, indent=2)
    try:
        subprocess.run(["git", "worktree", "add", "-b", branch, str(directory), base], check=True)
    except BaseException:
        claim.unlink()
        raise
    # Keep interrupted setup recoverable: after worktree creation the claim remains.
    # A shared exclude also supports task packets on branches older than this tool.
    exclude = claim.parent.parent.parent / "info" / "exclude"
    exclude.parent.mkdir(exist_ok=True)
    with exclude.open("a") as stream:
        stream.write("\n/.factory/\n")
    write_json(directory / ".factory/task.json", packet)
    (directory / ".factory/task.md").write_text(
        f"# {args.task}\n\nIssue: {args.issue}\n\nObjective: {args.objective}\n\n"
        f"Owner: {args.owner}\nBase: {base}\n\n"
        "Read AGENTS.md and docs/factory/README.md. Read the issue and epic before editing.\n"
        "Record the acceptance criteria, allowed files, dependencies and tests here before implementation.\n"
        "Use .factory/handoff.json for a harness switch; gate receipts are in .factory/checks/.\n"
        "Keep credentials, logs and decoded Secret values out of this packet.\n"
    )
    print(json.dumps(packet, indent=2))


def handoff(args):
    directory = root()
    packet = json.loads((directory / ".factory/task.json").read_text())
    data = {"task": packet["task"], "phase": args.phase, "summary": args.summary,
            "next": args.next, "blocker": args.blocker, "recorded_at": now(),
            "head_sha": git("rev-parse", "HEAD"), "fingerprint": fingerprint(directory)}
    with (directory / ".factory/events.jsonl").open("a") as stream:
        stream.write(json.dumps(data) + "\n")
    write_json(directory / ".factory/handoff.json", data)
    print(json.dumps(data, indent=2))


def status(_args):
    result = []
    for path in sorted(claims_dir().glob("*.json")):
        entry = {"task": path.stem}
        try:
            packet = json.loads(path.read_text())
            directory = Path(packet["worktree"])
            entry.update({"owner": packet["owner"], "worktree": str(directory), "exists": directory.is_dir()})
            handoff_path = directory / ".factory/handoff.json"
            if handoff_path.exists():
                entry["handoff"] = json.loads(handoff_path.read_text())
                entry["handoff_current"] = entry["handoff"]["fingerprint"] == fingerprint(directory)
        except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError):
            entry["error"] = "unreadable claim, handoff or worktree; inspect before recovery"
        result.append(entry)
    print(json.dumps(result, indent=2))


def release(args):
    packet = json.loads((root() / ".factory/task.json").read_text())
    claim = claims_dir() / f"{packet['task']}.json"
    current = json.loads(claim.read_text())
    if Path(current["worktree"]).resolve() != root().resolve():
        raise ValueError("claim belongs to another worktree")
    write_json(root() / ".factory/released.json", {"reason": args.reason, "at": now()})
    claim.unlink()
    print("Local claim released; worktree and branch retained. Update the Project separately.")


def prompt(args):
    directory = root()
    packet = json.loads((directory / ".factory/task.json").read_text())
    message = (
        f"Work only in {directory}. Task {packet['task']}: {packet['objective']}. "
        "Read AGENTS.md, docs/factory/README.md, .factory/task.md and any .factory/handoff.json. "
        "Verify the current HEAD and dirty diff against the handoff. "
    )
    if args.role == "review":
        message += (
            "Review the complete story diff against the packet base SHA and its acceptance criteria. "
            "Follow the contributor code-review reference. Record the exact reviewed SHA, model, "
            "findings and resolutions. Do not publish comments or merge."
        )
    else:
        message += (
            "Implement the next incomplete acceptance criterion, run the appropriate checks, "
            "and leave an explicit handoff with remaining work and evidence. "
            "A green fast gate is not completion. Do not publish or merge without task authorization."
        )
    if args.harness == "manual":
        print(message)
    else:
        executable = "codex" if args.harness == "codex" else "claude"
        command = [executable]
        if args.harness == "claude" and args.role == "review":
            command += ["--model", "claude-sonnet-5-5"]
        command.append(message)
        print(f"cd {shlex.quote(str(directory))} && {shlex.join(command)}")


def check(args):
    directory = root()
    config = json.loads((Path(__file__).parent / "config.json").read_text())
    commands = config["gates"][args.gate]
    if args.plan:
        print("\n".join(shlex.join(command) for command in commands))
        return
    receipt = {"gate": args.gate, "started_at": now(), "head_sha": git("rev-parse", "HEAD"),
               "before": fingerprint(directory), "commands": [], "passed": False}
    receipt_path = directory / ".factory/checks" / f"{time.time_ns()}.json"
    try:
        for command in commands:
            print(f"+ {shlex.join(command)}", flush=True)
            started = time.monotonic()
            result = run_command(command, cwd=directory, check=False)
            receipt["commands"].append({"argv": command, "exit_code": result.returncode,
                                        "elapsed_seconds": time.monotonic() - started})
            if result.returncode:
                break
        receipt["after"] = fingerprint(directory)
        receipt["passed"] = (len(receipt["commands"]) == len(commands)
                             and all(c["exit_code"] == 0 for c in receipt["commands"])
                             and receipt["before"] == receipt["after"])
    finally:
        receipt["finished_at"] = now()
        write_json(receipt_path, receipt)
        print(f"Receipt: {receipt_path}")
    if not receipt["passed"]:
        raise SystemExit(1)


def measure(args):
    if args.snapshot:
        if args.until or args.save:
            raise ValueError("snapshot replay uses its saved interval; --until and --save apply to collection")
        snapshot = json.loads(Path(args.snapshot).read_text())
    else:
        since = metrics.timestamp(args.since)
        until = metrics.timestamp(args.until) if args.until else dt.datetime.now(dt.timezone.utc)
        if until <= since:
            raise ValueError("until must be after since")
        if not re.fullmatch(r"[\w.-]+/[\w.-]+", args.repo):
            raise ValueError("repo must be owner/name")
        snapshot = metrics.collect(args.repo, since, until)
        if args.save:
            write_json(Path(args.save), snapshot)
    print(json.dumps(metrics.summary(snapshot), indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("metrics", help="read-only GitHub metrics, or replay a saved snapshot")
    source = p.add_mutually_exclusive_group(required=True)
    source.add_argument("--since", help="inclusive ISO timestamp with timezone")
    source.add_argument("--snapshot")
    p.add_argument("--until", help="exclusive ISO timestamp; defaults to now")
    p.add_argument("--repo", default="karan-vk/Oxikube")
    p.add_argument("--save", help="save sanitized raw snapshot")
    p.set_defaults(func=measure)
    p = sub.add_parser("start", help="claim a task locally and create an isolated worktree")
    for name in ("task", "slug", "worktree", "owner", "issue", "objective"):
        p.add_argument(f"--{name}", required=True)
    p.add_argument("--base", default="origin/main", help="fetch explicitly before starting")
    p.set_defaults(func=start)
    p = sub.add_parser("handoff", help="record current evidence and next action")
    p.add_argument("--phase", choices=["implement", "verify", "author-review", "pr-review", "blocked", "done"], required=True)
    p.add_argument("--summary", required=True)
    p.add_argument("--next", required=True)
    p.add_argument("--blocker")
    p.set_defaults(func=handoff)
    p = sub.add_parser("status", help="show local claims across this clone's worktrees")
    p.set_defaults(func=status)
    p = sub.add_parser("release", help="release this worktree's local claim; preserve files")
    p.add_argument("--reason", required=True)
    p.set_defaults(func=release)
    p = sub.add_parser("prompt", help="print a prompt or an interactive harness command; never launch")
    p.add_argument("--harness", choices=["codex", "claude", "manual"], default="manual")
    p.add_argument("--role", choices=["implement", "review"], default="implement")
    p.set_defaults(func=prompt)
    p = sub.add_parser("check", help="run configured gates and write revision-bound receipts")
    p.add_argument("--gate", choices=["fast", "full", "factory"], required=True)
    p.add_argument("--plan", action="store_true")
    p.set_defaults(func=check)
    args = parser.parse_args()
    try:
        args.func(args)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"factory: {error}\n")


if __name__ == "__main__":
    main()
