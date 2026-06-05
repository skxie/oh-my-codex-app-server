#!/usr/bin/env python3

import unittest
from pathlib import Path
import os
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))

import verify_layer1_ci


ROOT = Path(__file__).resolve().parents[1]
HOOK = ROOT / ".githooks" / "pre-push"
ZERO_OID = "0" * 40


def good_hook() -> str:
    branches = "|".join(
        verify_layer1_ci.hook_regex_branch(path)
        for path in verify_layer1_ci.REQUIRED_LAYER1_PATHS
    )
    return f"""#!/bin/sh
layer1_path_pattern='^({branches})'
if [ "${{LAYER1_PRE_PUSH_DRY_RUN:-}}" = "1" ]; then
    exit 0
fi
exec just pre-push-layer1
"""


def good_workflow() -> str:
    paths = "\n".join(
        f'      - "{verify_layer1_ci.workflow_path_token(path)}"'
        for path in verify_layer1_ci.REQUIRED_LAYER1_PATHS
    )
    return f"""name: runtime-layer1

on:
  pull_request:
    paths:
{paths}
  push:
    branches:
      - main
    paths:
{paths}
  workflow_dispatch:

jobs:
  pre-push-layer1:
    steps:
      - name: Run Layer 1 CI gate
        run: just pre-push-layer1-ci
"""


def good_justfile() -> str:
    return """pre-push-layer1-ci:
    just verify-layer1-ci
    just bazel-lock-check
    just fmt-check

pre-push-layer1:
    just verify-layer1-ci
    just bazel-lock-check
    just fmt-check
"""


class VerifyLayer1CiTests(unittest.TestCase):
    def test_good_fixture_validates(self) -> None:
        errors = verify_layer1_ci.validate_layer1_ci(
            good_hook(),
            good_workflow(),
            good_justfile(),
            hook_executable=True,
            enforce_executable=True,
        )

        self.assertEqual([], errors)

    def test_missing_push_path_fails_even_when_pull_request_has_it(self) -> None:
        missing_path = '      - "codex-rs/Cargo.lock"\n'
        workflow = good_workflow()
        first_index = workflow.find(missing_path)
        second_index = workflow.find(missing_path, first_index + len(missing_path))
        workflow = (
            workflow[:second_index] + workflow[second_index + len(missing_path) :]
        )

        errors = verify_layer1_ci.validate_layer1_ci(
            good_hook(),
            workflow,
            good_justfile(),
            hook_executable=True,
            enforce_executable=True,
        )

        self.assertIn(
            "runtime-layer1.yml push.paths is missing paths: codex-rs/Cargo.lock",
            errors,
        )

    def test_hook_pattern_must_match_layer1_samples(self) -> None:
        hook = good_hook().replace("codex-rs/runtime-api/", "codex-rs/runtime-apix/")

        errors = verify_layer1_ci.validate_layer1_ci(
            hook,
            good_workflow(),
            good_justfile(),
            hook_executable=True,
            enforce_executable=True,
        )

        self.assertIn(
            ".githooks/pre-push is missing paths: codex-rs/runtime-api/",
            errors,
        )

    def test_non_executable_hook_fails_on_unix(self) -> None:
        errors = verify_layer1_ci.validate_layer1_ci(
            good_hook(),
            good_workflow(),
            good_justfile(),
            hook_executable=False,
            enforce_executable=True,
        )

        self.assertIn(".githooks/pre-push must be executable", errors)

    def test_pre_push_layer1_must_run_bazel_lock_check(self) -> None:
        errors = verify_layer1_ci.validate_layer1_ci(
            good_hook(),
            good_workflow(),
            good_justfile().replace("    just bazel-lock-check\n", ""),
            hook_executable=True,
            enforce_executable=True,
        )

        self.assertIn(
            "pre-push-layer1 must run `just bazel-lock-check`",
            errors,
        )

    def test_workflow_must_run_ci_safe_gate(self) -> None:
        workflow = good_workflow().replace(
            "run: just pre-push-layer1-ci", "run: just pre-push-layer1"
        )

        errors = verify_layer1_ci.validate_layer1_ci(
            good_hook(),
            workflow,
            good_justfile(),
            hook_executable=True,
            enforce_executable=True,
        )

        self.assertIn(
            "runtime-layer1.yml must run `just pre-push-layer1-ci`",
            errors,
        )

    def test_ci_gate_must_not_run_full_app_server_suite(self) -> None:
        justfile = good_justfile().replace(
            "pre-push-layer1:\n",
            (
                "    RUST_MIN_STACK=16777216 cargo nextest run --no-fail-fast "
                "-p codex-app-server\n\npre-push-layer1:\n"
            ),
        )

        errors = verify_layer1_ci.validate_layer1_ci(
            good_hook(),
            good_workflow(),
            justfile,
            hook_executable=True,
            enforce_executable=True,
        )

        self.assertIn(
            "pre-push-layer1-ci must not run the full codex-app-server suite",
            errors,
        )


class PrePushHookTests(unittest.TestCase):
    def run_git(self, repo: Path, *args: str) -> str:
        result = subprocess.run(
            ["git", *args],
            cwd=repo,
            text=True,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        return result.stdout.strip()

    def commit_all(self, repo: Path, message: str) -> str:
        self.run_git(repo, "add", ".")
        self.run_git(
            repo,
            "-c",
            "user.email=layer1@example.com",
            "-c",
            "user.name=Layer One",
            "commit",
            "-m",
            message,
        )
        return self.run_git(repo, "rev-parse", "HEAD")

    def make_repo(self, *, branch: str = "main") -> Path:
        tempdir = tempfile.TemporaryDirectory()
        self.addCleanup(tempdir.cleanup)
        repo = Path(tempdir.name)
        self.run_git(repo, "init", "-q", "-b", branch)
        return repo

    def run_hook(self, repo: Path, stdin: str) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["LAYER1_PRE_PUSH_DRY_RUN"] = "1"
        return subprocess.run(
            [str(HOOK)],
            cwd=repo,
            input=stdin,
            text=True,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
        )

    def test_hook_triggers_for_layer1_path_diff_on_non_layer_ref(self) -> None:
        repo = self.make_repo()
        (repo / "README.md").write_text("base\n", encoding="utf-8")
        base = self.commit_all(repo, "base")
        (repo / "codex-rs").mkdir()
        (repo / "codex-rs" / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        head = self.commit_all(repo, "layer1 path")

        result = self.run_hook(
            repo, f"refs/heads/topic {head} refs/heads/topic {base}\n"
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("would run: just pre-push-layer1", result.stdout)

    def test_hook_skips_non_layer_path_diff_on_non_layer_ref(self) -> None:
        repo = self.make_repo()
        (repo / "README.md").write_text("base\n", encoding="utf-8")
        base = self.commit_all(repo, "base")
        (repo / "README.md").write_text("changed\n", encoding="utf-8")
        head = self.commit_all(repo, "non layer path")

        result = self.run_hook(
            repo, f"refs/heads/topic {head} refs/heads/topic {base}\n"
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("gate skipped", result.stdout)

    def test_hook_skips_branch_delete_even_for_layer1_ref(self) -> None:
        repo = self.make_repo()
        (repo / "README.md").write_text("base\n", encoding="utf-8")
        head = self.commit_all(repo, "base")

        result = self.run_hook(
            repo,
            f"refs/heads/codex/runtime-extension-layer1 {ZERO_OID} "
            f"refs/heads/codex/runtime-extension-layer1 {head}\n",
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("gate skipped", result.stdout)

    def test_hook_runs_for_new_root_branch_without_merge_base(self) -> None:
        repo = self.make_repo(branch="topic")
        (repo / "codex-rs").mkdir()
        (repo / "codex-rs" / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        head = self.commit_all(repo, "root layer1 path")

        result = self.run_hook(
            repo, f"refs/heads/topic {head} refs/heads/topic {ZERO_OID}\n"
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("would run: just pre-push-layer1", result.stdout)


if __name__ == "__main__":
    unittest.main()
