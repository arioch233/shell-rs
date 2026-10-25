#!/usr/bin/env python3
"""在临时仓库验证同步脚本，不访问网络或修改当前仓库"""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("sync-upstream.sh").resolve()


class SyncUpstreamTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="shellrs-sync-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.output = self.root / "output"
        self.summary = self.root / "summary"
        self.git("init", "-b", "main")
        self.git("config", "user.name", "Test")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.base = self.commit("shared.txt", "base\n")
        self.ref("origin/main", self.base)
        self.ref("upstream/main", self.base)

    def git(self, *args):
        return subprocess.run(
            ["git", *args], cwd=self.repo, check=True,
            capture_output=True, text=True,
        ).stdout.strip()

    def ref(self, name, commit):
        self.git("update-ref", f"refs/remotes/{name}", commit)

    def commit(self, name, content):
        (self.repo / name).write_text(content)
        self.git("add", name)
        self.git("commit", "-m", f"Update {name}")
        return self.git("rev-parse", "HEAD")

    def run_sync(self, success=True):
        self.output.write_text("")
        self.summary.write_text("")
        result = subprocess.run(
            ["bash", str(SCRIPT)], cwd=self.repo, capture_output=True, text=True,
            env={**os.environ, "GITHUB_OUTPUT": str(self.output),
                 "GITHUB_STEP_SUMMARY": str(self.summary)},
        )
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return self.output.read_text()

    def diverge(self):
        custom = self.commit("custom.txt", "custom\n")
        self.ref("origin/main", custom)
        self.git("switch", "--detach", self.base)
        upstream = self.commit("upstream.txt", "upstream\n")
        self.ref("upstream/main", upstream)
        return custom, upstream

    def test_no_updates(self):
        before = self.git("rev-parse", "HEAD")
        self.assertIn("needs_pr=false", self.run_sync())
        self.assertEqual(before, self.git("rev-parse", "HEAD"))

    def test_merge_keeps_both_histories(self):
        custom, upstream = self.diverge()
        self.assertIn("needs_pr=true", self.run_sync())
        self.git("merge-base", "--is-ancestor", custom, "HEAD")
        self.git("merge-base", "--is-ancestor", upstream, "HEAD")
        self.assertEqual(len(self.git("show", "-s", "--format=%P").split()), 2)
        self.assertEqual((self.repo / "custom.txt").read_text(), "custom\n")

    def test_repeat_preserves_head_and_manual_commits(self):
        self.diverge()
        self.run_sync()
        manual = self.commit("manual.txt", "manual resolution\n")
        self.ref("origin/sync/upstream-update", manual)
        self.assertIn("needs_pr=true", self.run_sync())
        self.assertEqual(manual, self.git("rev-parse", "HEAD"))
        # 自己的 main 前进后，同步必须保留之前的人工提交
        self.git("switch", "--detach", "origin/main")
        latest = self.commit("later.txt", "later custom\n")
        self.ref("origin/main", latest)
        self.run_sync()
        self.git("merge-base", "--is-ancestor", manual, "HEAD")
        self.git("merge-base", "--is-ancestor", latest, "HEAD")

    def test_conflict_does_not_change_remote_branch(self):
        custom = self.commit("shared.txt", "custom\n")
        self.ref("origin/main", custom)
        self.ref("origin/sync/upstream-update", custom)
        self.git("switch", "--detach", self.base)
        upstream = self.commit("shared.txt", "upstream\n")
        self.ref("upstream/main", upstream)
        self.assertNotIn("needs_pr=true", self.run_sync(success=False))
        self.assertIn("shared.txt", self.summary.read_text())
        self.assertEqual(custom, self.git("rev-parse", "origin/sync/upstream-update"))
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertFalse((self.repo / ".git" / "MERGE_HEAD").exists())

    def test_dirty_worktree_is_preserved(self):
        (self.repo / "shared.txt").write_text("uncommitted\n")
        self.run_sync(success=False)
        self.assertEqual((self.repo / "shared.txt").read_text(), "uncommitted\n")


if __name__ == "__main__":
    unittest.main()
