set working-directory := "codex-rs"
set positional-arguments
export JUST_SHELL := justfile_directory() / "scripts/just-shell.py"
set shell := ["python3", "-c", 'import os, runpy; runpy.run_path(os.environ["JUST_SHELL"], run_name="__main__")']
set windows-shell := ["python", "-c", 'import os, runpy; runpy.run_path(os.environ["JUST_SHELL"], run_name="__main__")']

rust_min_stack := "8388608" # 8 MiB
python := if os_family() == "windows" { "python" } else { "python3" }

# Display help
help:
    just -l

# `codex`
alias c := codex
codex *args:
    cargo run --bin codex -- {args}

# `codex exec`
exec *args:
    cargo run --bin codex -- exec {args}

# Start `codex exec-server` and run codex-tui.
[no-cd]
[positional-arguments]
[unix]
tui-with-exec-server *args:
    {{ justfile_directory() }}/scripts/run_tui_with_exec_server.sh "$@"

# Run the CLI version of the file-search crate.
file-search *args:
    cargo run --bin codex-file-search -- {args}

# Run the standalone code-mode host from source.
code-mode-host *args:
    cargo run --bin codex-code-mode-host -- {args}

# Build the CLI and run the app-server test client
app-server-test-client *args:
    cargo build -p codex-cli
    cargo run -p codex-app-server-test-client -- --codex-bin ./target/debug/codex {args}

# Format the justfile, Rust, Bazel/Starlark, Python SDK code, and Python scripts.
fmt:
    @{{ python }} ../scripts/format.py

# Check formatting without modifying files.
fmt-check:
    @{{ python }} ../scripts/format.py --check

fix *args:
    cargo clippy --fix --tests --allow-dirty {args}

clippy *args:
    cargo clippy --tests {args}

[unix]
install:
    rustup show active-toolchain
    cargo fetch

[windows]
install:
    #!powershell.exe -File
    $pwsh = Get-Command pwsh.exe -ErrorAction SilentlyContinue
    if (-not $pwsh) {
        winget install --exact --id Microsoft.PowerShell --source winget --accept-package-agreements --accept-source-agreements
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    rustup show active-toolchain
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    cargo fetch
    exit $LASTEXITCODE

# Run nextest with --no-fail-fast so all tests are run.
#
# Run `cargo install --locked cargo-nextest` if you don't have it installed.
# Prefer this for routine local runs. Workspace crate features are banned, so
# there should be no need to add `--all-features`.
[unix]
test *args:
    RUST_MIN_STACK={{ rust_min_stack }} NEXTEST_PROFILE=local cargo nextest run --no-fail-fast "$@"

[windows]
test *args:
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; $env:NEXTEST_PROFILE = "local"; cargo nextest run --no-fail-fast @($args | Select-Object -Skip 1)

# Warm-checkout Layer 1 quickstart for downstream runtime extension authors.
[unix]
tthw-layer1:
    start=$(date +%s); RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast --no-capture -p codex-app-server runtime_registry_fake_backend_fixture_takes_effect_through_in_process_app_server; status=$?; end=$(date +%s); elapsed=$((end - start)); echo "tthw-layer1 elapsedSeconds=$elapsed warmTargetSeconds=300"; exit $status

[windows]
tthw-layer1:
    $start = Get-Date; $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast --no-capture -p codex-app-server runtime_registry_fake_backend_fixture_takes_effect_through_in_process_app_server; $status = $LASTEXITCODE; $elapsed = [int]((Get-Date) - $start).TotalSeconds; Write-Host "tthw-layer1 elapsedSeconds=$elapsed warmTargetSeconds=300"; exit $status

# Validate the local push hook and GitHub workflow wiring for Layer 1.
[no-cd]
[unix]
verify-layer1-ci:
    sh -n {{ justfile_directory() }}/.githooks/pre-push
    {{ python }} {{ justfile_directory() }}/scripts/test_verify_layer1_ci.py
    {{ python }} {{ justfile_directory() }}/scripts/verify_layer1_ci.py
    printf '%s %s %s %s\n' refs/heads/codex/runtime-extension-layer1 "$(git rev-parse HEAD)" refs/heads/codex/runtime-extension-layer1 0000000000000000000000000000000000000000 | LAYER1_PRE_PUSH_DRY_RUN=1 {{ justfile_directory() }}/.githooks/pre-push

[no-cd]
[windows]
verify-layer1-ci:
    {{ python }} {{ justfile_directory() }}/scripts/test_verify_layer1_ci.py
    {{ python }} {{ justfile_directory() }}/scripts/verify_layer1_ci.py

# Rebase guard for the fork-owned Layer 1 runtime seams.
[unix]
verify-layer1-adapters:
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-runtime-api
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-api process_sse_exposes_raw_provider_usage_metadata
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-core runtime_model_request_adapter runtime_context runtime_tool_middleware runtime_usage_metadata_mapper_changes_recorded_token_usage
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-app-server-sdk
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-app-server layer2_cookbook_examples
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-app-server runtime_registry_fake_backend_fixture
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-memories-write

[windows]
verify-layer1-adapters:
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-runtime-api
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-api process_sse_exposes_raw_provider_usage_metadata
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-core runtime_model_request_adapter runtime_context runtime_tool_middleware runtime_usage_metadata_mapper_changes_recorded_token_usage
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-app-server-sdk
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-app-server layer2_cookbook_examples
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-app-server runtime_registry_fake_backend_fixture
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-memories-write

# Push-time gate for the fork-owned Layer 1 runtime extension branch.
[unix]
pre-push-layer1:
    just verify-layer1-ci
    just fmt-check
    just verify-layer1-adapters
    just tthw-layer1
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-api
    RUST_MIN_STACK={{ rust_min_stack }} cargo nextest run --no-fail-fast -p codex-app-server
    just bench-smoke

[windows]
pre-push-layer1:
    just verify-layer1-ci
    just fmt-check
    just verify-layer1-adapters
    just tthw-layer1
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-api
    $env:RUST_MIN_STACK = "{{ rust_min_stack }}"; cargo nextest run --no-fail-fast -p codex-app-server
    just bench-smoke

# Run from the repository root so scripts that resolve paths from `cwd` see
# the same layout they use in GitHub Actions.
[no-cd]
test-github-scripts:
    {{ python }} -m unittest discover -s {{ justfile_directory() }}/.github/scripts -p 'test_*.py'

# Run explicit workspace benchmark targets.
bench *args:
    cargo bench --workspace --bench '*' {args}

# Run benchmark targets once to ensure they start successfully.
bench-smoke:
    just bench -- --test

# Build and run Codex from source using Bazel.
# On Unix, use `[no-cd]` and `--run_under="cd $PWD &&"` to ensure Bazel runs
# the command in the current working directory.
[no-cd]
[unix]
bazel-codex *args:
    bazel run //codex-rs/cli:codex --run_under="cd $PWD &&" -- "$@"

[windows]
bazel-codex *args:
    bazel run //codex-rs/cli:codex --run_under='cd /d "{{ invocation_directory_native() }}" &&' -- @($args | Select-Object -Skip 1)

# Build and run the standalone code-mode host from source using Bazel.
[no-cd]
[unix]
bazel-code-mode-host *args:
    bazel run //codex-rs/code-mode-host:codex-code-mode-host --run_under="cd $PWD &&" -- "$@"

[windows]
bazel-code-mode-host *args:
    bazel run //codex-rs/code-mode-host:codex-code-mode-host --run_under='cd /d "{{ invocation_directory_native() }}" &&' -- @($args | Select-Object -Skip 1)

[no-cd]
bazel-lock-update:
    bazel mod deps --lockfile_mode=update

[no-cd]
[unix]
bazel-lock-check:
    {{ justfile_directory() }}/scripts/check-module-bazel-lock.sh

[windows]
bazel-lock-check:
    bazel mod deps --lockfile_mode=error; if ($LASTEXITCODE -ne 0) { Write-Error "MODULE.bazel.lock is out of date. Run 'just bazel-lock-update' and commit the updated lockfile."; exit 1 }

bazel-test:
    bazel test --test_tag_filters=-argument-comment-lint //... --keep_going

[no-cd]
[unix]
bazel-clippy:
    bazel_targets="$({{ justfile_directory() }}/scripts/list-bazel-clippy-targets.sh)" && bazel build --config=clippy -- ${bazel_targets}

[no-cd]
[unix]
bazel-argument-comment-lint:
    bazel build --config=argument-comment-lint -- $({{ justfile_directory() }}/tools/argument-comment-lint/list-bazel-targets.sh)

build-for-release:
    bazel build //codex-rs/cli:release_binaries

# Run the MCP server
mcp-server-run *args:
    cargo run -p codex-mcp-server -- {args}

# Regenerate the json schema for config.toml from the current config types.
write-config-schema:
    cargo run -p codex-core --bin codex-write-config-schema

# Regenerate vendored app-server protocol schema artifacts.
write-app-server-schema *args:
    cargo run -p codex-app-server-protocol --bin write_schema_fixtures -- {args}

[no-cd]
write-hooks-schema:
    cargo run --manifest-path {{ justfile_directory() }}/codex-rs/Cargo.toml -p codex-hooks --bin write_hooks_schema_fixtures

# Run the argument-comment Dylint checks across codex-rs.
[no-cd]
[unix]
argument-comment-lint *args:
    if [ "$#" -eq 0 ]; then \
      bazel build --config=argument-comment-lint -- $({{ justfile_directory() }}/tools/argument-comment-lint/list-bazel-targets.sh); \
    else \
      {{ justfile_directory() }}/tools/argument-comment-lint/run-prebuilt-linter.py "$@"; \
    fi

[no-cd]
argument-comment-lint-from-source *args:
    {{ python }} {{ justfile_directory() }}/tools/argument-comment-lint/run.py {args}

# Tail logs from the state SQLite database
[unix]
log *args:
    if [ "${1:-}" = "--" ]; then shift; fi; cargo run -p codex-state --bin logs_client -- "$@"

[windows]
log *args:
    $forwarded_args = @($args | Select-Object -Skip 1); if ($forwarded_args.Count -gt 0 -and $forwarded_args[0] -eq "--") { $forwarded_args = @($forwarded_args | Select-Object -Skip 1) }; cargo run -p codex-state --bin logs_client -- @forwarded_args
