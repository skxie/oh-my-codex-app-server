#!/usr/bin/env python3
"""Validate fork-owned Layer 1 CI and pre-push hook wiring."""

import os
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
HOOK = ROOT / ".githooks" / "pre-push"
WORKFLOW = ROOT / ".github" / "workflows" / "runtime-layer1.yml"
JUSTFILE = ROOT / "justfile"

REQUIRED_LAYER1_PATHS = [
    "codex-rs/runtime-api/",
    "codex-rs/core/",
    "codex-rs/codex-api/",
    "codex-rs/app-server/",
    "codex-rs/app-server-client/",
    "codex-rs/app-server-sdk/",
    "codex-rs/memories/write/",
    "codex-rs/Cargo.toml",
    "codex-rs/Cargo.lock",
    "MODULE.bazel.lock",
    "justfile",
    "scripts/verify_layer1_ci.py",
    "scripts/test_verify_layer1_ci.py",
    ".githooks/pre-push",
    ".github/workflows/runtime-layer1.yml",
]


def fail(message: str) -> None:
    print(f"Layer 1 CI wiring check failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: Path) -> str:
    if not path.is_file():
        fail(f"missing required file: {path.relative_to(ROOT)}")
    return path.read_text(encoding="utf-8")


def hook_regex_branch(path: str) -> str:
    if path.endswith("/"):
        return path
    return path.replace(".", r"\.") + "$"


def workflow_path_token(path: str) -> str:
    if path.endswith("/"):
        return f"{path}**"
    return path


def hook_sample_path(path: str) -> str:
    if path.endswith("/"):
        return f"{path}sample.rs"
    return path


def extract_hook_path_pattern(hook: str) -> str | None:
    match = re.search(r"^layer1_path_pattern='([^']+)'$", hook, re.MULTILINE)
    if match is None:
        return None
    return match.group(1)


def extract_workflow_event_paths(workflow: str, event: str) -> list[str]:
    lines = workflow.splitlines()
    event_start = None
    event_indent = None

    for index, line in enumerate(lines):
        match = re.match(rf"^(\s*){re.escape(event)}:\s*$", line)
        if match is not None:
            event_start = index
            event_indent = len(match.group(1))
            break

    if event_start is None or event_indent is None:
        return []

    event_block: list[str] = []
    for line in lines[event_start + 1 :]:
        if line.strip() and len(line) - len(line.lstrip()) <= event_indent:
            break
        event_block.append(line)

    paths_start = None
    paths_indent = None
    for index, line in enumerate(event_block):
        match = re.match(r"^(\s*)paths:\s*$", line)
        if match is not None:
            paths_start = index
            paths_indent = len(match.group(1))
            break

    if paths_start is None or paths_indent is None:
        return []

    paths = []
    for line in event_block[paths_start + 1 :]:
        if line.strip() and len(line) - len(line.lstrip()) <= paths_indent:
            break
        match = re.match(r'^\s*-\s+"([^"]+)"\s*$', line)
        if match is not None:
            paths.append(match.group(1))
    return paths


def validate_layer1_ci(
    hook: str,
    workflow: str,
    justfile: str,
    *,
    hook_executable: bool,
    enforce_executable: bool,
) -> list[str]:
    errors = []

    if not hook.startswith("#!/bin/sh\n"):
        errors.append(".githooks/pre-push must start with a /bin/sh shebang")
    if enforce_executable and not hook_executable:
        errors.append(".githooks/pre-push must be executable")
    if "exec just pre-push-layer1" not in hook:
        errors.append(".githooks/pre-push must execute `just pre-push-layer1`")
    if "LAYER1_PRE_PUSH_DRY_RUN" not in hook:
        errors.append(".githooks/pre-push must support LAYER1_PRE_PUSH_DRY_RUN")

    if "run: just pre-push-layer1-ci" not in workflow:
        errors.append("runtime-layer1.yml must run `just pre-push-layer1-ci`")

    if "just bazel-lock-check" not in justfile:
        errors.append("pre-push-layer1 must run `just bazel-lock-check`")
    if "pre-push-layer1-ci:" not in justfile:
        errors.append("justfile must define `pre-push-layer1-ci`")
    ci_gate = justfile.partition("pre-push-layer1-ci:")[2].partition(
        "pre-push-layer1:"
    )[0]
    if "cargo nextest run --no-fail-fast -p codex-app-server\n" in ci_gate:
        errors.append("pre-push-layer1-ci must not run the full codex-app-server suite")
    for required in (
        "-p codex-app-server layer2_cookbook_examples",
        "-p codex-app-server runtime_registry_fake_backend_fixture",
    ):
        if required not in ci_gate:
            errors.append(f"pre-push-layer1-ci must run `{required}`")
    for forbidden in (
        "just verify-layer1-adapters",
        "just bazel-lock-check",
        "just fmt-check",
        "just bench-smoke",
        "-p codex-api",
        "-p codex-core",
        "-p codex-runtime-api",
        "-p codex-app-server-sdk",
        "-p codex-memories-write",
    ):
        if forbidden in ci_gate:
            errors.append(f"pre-push-layer1-ci must not run `{forbidden}`")

    hook_pattern = extract_hook_path_pattern(hook)
    if hook_pattern is None:
        errors.append(".githooks/pre-push must define layer1_path_pattern")
    else:
        try:
            compiled_hook_pattern = re.compile(hook_pattern)
        except re.error as exc:
            errors.append(f".githooks/pre-push layer1_path_pattern is invalid: {exc}")
        else:
            missing_hook_paths = [
                path
                for path in REQUIRED_LAYER1_PATHS
                if compiled_hook_pattern.search(hook_sample_path(path)) is None
            ]
            if missing_hook_paths:
                errors.append(
                    ".githooks/pre-push is missing paths: "
                    + ", ".join(missing_hook_paths)
                )

            if compiled_hook_pattern.search("docs/not-layer1.md") is not None:
                errors.append(".githooks/pre-push matches unrelated paths")

    workflow_paths_by_event = {
        event: extract_workflow_event_paths(workflow, event)
        for event in ("pull_request", "push")
    }
    for event, paths in workflow_paths_by_event.items():
        missing_paths = [
            path
            for path in REQUIRED_LAYER1_PATHS
            if workflow_path_token(path) not in paths
        ]
        if missing_paths:
            errors.append(
                f"runtime-layer1.yml {event}.paths is missing paths: "
                + ", ".join(missing_paths)
            )

    return errors


def main() -> int:
    hook = read(HOOK)
    workflow = read(WORKFLOW)
    justfile = read(JUSTFILE)
    errors = validate_layer1_ci(
        hook,
        workflow,
        justfile,
        hook_executable=os.access(HOOK, os.X_OK),
        enforce_executable=os.name != "nt",
    )
    if errors:
        fail("; ".join(errors))

    print("runtime-layer1 ci wiring ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
