"""Deterministic tests using disposable Git repositories and mocked GitHub pages."""

import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import factory
import metrics


def run(number, conclusion="success", event="pull_request", attempt=1):
    return {"id": number, "name": "ci", "path": ".github/workflows/ci.yml", "event": event,
            "head_sha": "abc", "head_branch": "story/test", "run_attempt": attempt,
            "status": "completed", "conclusion": conclusion,
            "created_at": "2026-10-04T00:00:00Z", "updated_at": "2026-10-04T00:20:00Z",
            "html_url": "https://example.test/run"}


def pull(number, merged="2026-10-04T01:00:00Z"):
    return {"number": number, "title": "story", "state": "closed" if merged else "open",
            "draft": False, "created_at": "2026-10-04T00:00:00Z",
            "updated_at": "2026-10-04T01:00:00Z", "merged_at": merged,
            "closed_at": merged, "html_url": "https://example.test/pr"}


class MetricsTests(unittest.TestCase):
    def test_cohorts_exclude_unmerged_and_cutoff_is_exclusive(self):
        data = {"schema": 1, "repo": "a/b", "since": "2026-10-04T00:00:00Z",
                "until": "2026-10-05T00:00:00Z",
                "pulls": [pull(1), pull(2, None), pull(3, "2026-10-05T00:00:00Z")],
                "runs": [run(1), run(2, "cancelled"), run(3, "failure", attempt=2), run(4, event="push")]}
        result = metrics.summary(data)
        self.assertEqual(result["merged_prs"], 1)
        self.assertEqual(result["merged_per_24h"], 1)
        self.assertEqual(result["open_prs_at_collection"], 1)
        self.assertEqual(result["pr_open_to_merge_hours_p50"], 1)
        pr = result["workflows"][".github/workflows/ci.yml (pull_request)"]
        self.assertEqual(pr["conclusions"], {"success": 1, "cancelled": 1, "failure": 1})
        self.assertEqual(pr["additional_attempts"], 1)

    def test_empty_percentiles_and_nearest_rank(self):
        self.assertIsNone(metrics.percentile([], .5))
        self.assertEqual(metrics.percentile([1, 2, 3, 4], .5), 2)
        self.assertEqual(metrics.percentile([1, 2, 3, 4], .9), 4)

    def test_timezone_required(self):
        with self.assertRaises(ValueError):
            metrics.timestamp("2026-10-04")

    def test_snapshot_replay_rejects_ignored_interval_override(self):
        with self.assertRaisesRegex(ValueError, "saved interval"):
            factory.measure(SimpleNamespace(snapshot="snapshot.json", until="2026-10-05T00:00:00Z", save=None))

    def test_collect_paginates_and_retains_old_open_prs(self):
        old = pull(101, None)
        old["updated_at"] = "2020-01-01T00:00:00Z"
        pages = [[pull(i) for i in range(100)], [old],
                 {"total_count": 101, "workflow_runs": [run(i) for i in range(100)]},
                 {"total_count": 101, "workflow_runs": [run(101)]}]
        with patch.object(metrics, "api", side_effect=pages) as api:
            result = metrics.collect("a/b", metrics.timestamp("2026-10-04T00:00:00Z"), metrics.timestamp("2026-10-05T00:00:00Z"))
        self.assertEqual(len(result["runs"]), 101)
        self.assertEqual(len(result["pulls"]), 101)
        self.assertIn("page=2", api.call_args_list[1].args[0])
        self.assertIn("page=2", api.call_args_list[3].args[0])
        self.assertNotIn("body", result["pulls"][0])

    def test_collection_refuses_truncated_run_day(self):
        with patch.object(metrics, "api", side_effect=[[], {"total_count": 1000}]):
            with self.assertRaisesRegex(ValueError, ">=1000"):
                metrics.collect("a/b", metrics.timestamp("2026-10-04T00:00:00Z"), metrics.timestamp("2026-10-05T00:00:00Z"))

    def test_daily_shards_and_boundary_deduplication(self):
        with patch.object(metrics, "api", side_effect=[[], {"total_count": 1, "workflow_runs": [run(1)]}, {"total_count": 1, "workflow_runs": [run(1)]}]) as api:
            result = metrics.collect("a/b", metrics.timestamp("2026-10-04T00:00:00Z"), metrics.timestamp("2026-10-06T00:00:00Z"))
        self.assertEqual(len(result["runs"]), 1)
        self.assertIn("created=2026-10-04", api.call_args_list[1].args[0])
        self.assertIn("created=2026-10-05", api.call_args_list[2].args[0])


class WorktreeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.repo = self.directory / "repo"
        self.repo.mkdir()
        self.previous = Path.cwd()
        os.chdir(self.repo)
        self.addCleanup(os.chdir, self.previous)
        subprocess.run(["git", "init", "-q"], check=True)
        (self.repo / ".gitignore").write_text("/.factory/\n")
        (self.repo / "source").write_text("original\n")
        factory.git("add", ".")
        factory.git("-c", "user.name=Test", "-c", "user.email=test@example.test", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", "commit", "-qm", "initial")
        self.args = SimpleNamespace(task="E01-S99", slug="test", worktree=str(self.directory / "task"),
                                    base="HEAD", issue="https://example.test/1", objective="test task", owner="codex")

    def start(self):
        with contextlib.redirect_stdout(io.StringIO()):
            factory.start(self.args)

    def test_duplicate_claim_is_rejected_and_handoff_survives_harness_switch(self):
        self.start()
        with self.assertRaises(FileExistsError):
            self.start()
        os.chdir(self.args.worktree)
        with contextlib.redirect_stdout(io.StringIO()):
            factory.handoff(SimpleNamespace(phase="verify", summary="implemented", next="run gate", blocker=None))
        data = json.loads(Path(".factory/handoff.json").read_text())
        self.assertEqual(data["head_sha"], factory.git("rev-parse", "HEAD"))
        with contextlib.redirect_stdout(io.StringIO()):
            factory.handoff(SimpleNamespace(phase="blocked", summary="check failed", next="repair", blocker="test failure"))
        history = [json.loads(line) for line in Path(".factory/events.jsonl").read_text().splitlines()]
        self.assertEqual([event["phase"] for event in history], ["verify", "blocked"])
        self.assertEqual(factory.git("status", "--porcelain"), "")
        for harness in ["codex", "claude", "manual"]:
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                factory.prompt(SimpleNamespace(harness=harness, role="review"))
            self.assertIn(".factory/handoff.json", output.getvalue())
        with contextlib.redirect_stdout(io.StringIO()):
            factory.release(SimpleNamespace(reason="handoff complete"))
        self.assertFalse((factory.claims_dir() / "E01-S99.json").exists())
        self.assertTrue(Path("source").exists())

    def test_failed_worktree_setup_releases_claim(self):
        self.args.worktree = str(self.repo)
        with self.assertRaises(subprocess.CalledProcessError):
            self.start()
        self.assertFalse((factory.claims_dir() / "E01-S99.json").exists())

    def test_status_keeps_healthy_claims_when_another_claim_is_corrupt(self):
        self.start()
        (factory.claims_dir() / "broken.json").write_text("")
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            factory.status(None)
        entries = {entry["task"]: entry for entry in json.loads(output.getvalue())}
        self.assertTrue(entries["E01-S99"]["exists"])
        self.assertIn("error", entries["broken"])

    def test_fingerprint_tracks_staged_unstaged_and_untracked_not_receipts(self):
        initial = factory.fingerprint(self.repo)
        (self.repo / "source").write_text("changed\n")
        changed = factory.fingerprint(self.repo)
        self.assertNotEqual(initial, changed)
        factory.git("add", "source")
        self.assertEqual(changed, factory.fingerprint(self.repo))
        (self.repo / "new-file").write_text("new\n")
        untracked = factory.fingerprint(self.repo)
        self.assertNotEqual(changed, untracked)
        factory.write_json(self.repo / ".factory/checks/result.json", {"passed": True})
        self.assertEqual(untracked, factory.fingerprint(self.repo))
        (self.repo / "new-file").chmod(0o755)
        self.assertNotEqual(untracked, factory.fingerprint(self.repo))

    def test_failed_gate_records_failure_and_stops(self):
        with patch.object(factory, "run_command", return_value=SimpleNamespace(returncode=7)) as execute:
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
                factory.check(SimpleNamespace(gate="full", plan=False))
        self.assertEqual(execute.call_count, 1)
        receipt = json.loads(next(Path(".factory/checks").glob("*.json")).read_text())
        self.assertFalse(receipt["passed"])
        self.assertEqual(receipt["commands"][0]["exit_code"], 7)

    def test_edit_during_successful_gate_invalidates_receipt(self):
        def edit(*args, **kwargs):
            (self.repo / "source").write_text("changed during test\n")
            return SimpleNamespace(returncode=0)
        with patch.object(factory, "run_command", side_effect=edit):
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
                factory.check(SimpleNamespace(gate="factory", plan=False))
        receipt = json.loads(next(Path(".factory/checks").glob("*.json")).read_text())
        self.assertFalse(receipt["passed"])

    def test_successful_gate_records_exact_revision(self):
        with patch.object(factory, "run_command", return_value=SimpleNamespace(returncode=0)):
            with contextlib.redirect_stdout(io.StringIO()):
                factory.check(SimpleNamespace(gate="factory", plan=False))
        receipt = json.loads(next(Path(".factory/checks").glob("*.json")).read_text())
        self.assertTrue(receipt["passed"])
        self.assertEqual(receipt["head_sha"], factory.git("rev-parse", "HEAD"))

    def test_interrupted_gate_keeps_failed_receipt(self):
        with patch.object(factory, "run_command", side_effect=KeyboardInterrupt):
            with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(KeyboardInterrupt):
                factory.check(SimpleNamespace(gate="factory", plan=False))
        receipt = json.loads(next(Path(".factory/checks").glob("*.json")).read_text())
        self.assertFalse(receipt["passed"])
        self.assertIn("finished_at", receipt)


if __name__ == "__main__":
    unittest.main()
