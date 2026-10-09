#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Darwin) ;;
  *) echo "The Rust coverage pilot requires macOS" >&2; exit 1 ;;
esac

toolchain=1.93.1
test_filters='commands::claude_config::tests:: commands::github::tests:: commands::code_server::view::tests:: menu::'
report_dir=coverage/rust/macos
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/worktreemanager-target}"
export CARGO_LLVM_COV_TARGET_DIR="${CARGO_LLVM_COV_TARGET_DIR:-$CARGO_TARGET_DIR/llvm-cov-pilot}"
export CARGO_TARGET_DIR="$CARGO_LLVM_COV_TARGET_DIR"
case " ${RUSTFLAGS:-} " in
  *' --remap-path-prefix=tests/../src=src '*) ;;
  *) export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=tests/../src=src" ;;
esac

rm -rf "$report_dir"
version=$(rustup run "$toolchain" cargo llvm-cov --version)
if [ "$version" != 'cargo-llvm-cov 0.9.1' ]; then
  echo "Expected cargo-llvm-cov 0.9.1, got $version" >&2
  exit 1
fi

mkdir -p "$report_dir"
rustup run "$toolchain" rustc -Vv > "$report_dir/toolchain.txt"
echo "$version" >> "$report_dir/toolchain.txt"
echo "$test_filters --test menu" >> "$report_dir/toolchain.txt"

rustup run "$toolchain" cargo llvm-cov clean --workspace --manifest-path src-tauri/Cargo.toml
coverage_env=$(rustup run "$toolchain" cargo llvm-cov show-env --sh --manifest-path src-tauri/Cargo.toml)
test_status=0
(
  eval "$coverage_env"
  /usr/bin/time -p -o "$report_dir/instrumented-build.txt" \
    rustup run "$toolchain" cargo test --locked --manifest-path src-tauri/Cargo.toml \
      --lib --test menu --no-run --message-format=json-render-diagnostics \
      > "$report_dir/build.jsonl" || exit $?
  /usr/bin/time -p -o "$report_dir/instrumented-test.txt" \
    python3 - "$report_dir/build.jsonl" $test_filters <<'PY'
import json
import subprocess
import sys

executables = {}
with open(sys.argv[1]) as messages:
    for line in messages:
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") != "compiler-artifact" or not message.get("executable"):
            continue
        target = message["target"]
        if target["name"] == "app_lib" and message["profile"]["test"]:
            executables["unit"] = message["executable"]
        elif target["name"] == "menu" and target["kind"] == ["test"]:
            executables["menu"] = message["executable"]

if executables.keys() != {"unit", "menu"}:
    sys.exit("Expected the app_lib unit tests and native menu test executable")

statuses = [
    subprocess.run([executables["unit"], *sys.argv[2:]], cwd="src-tauri").returncode,
    subprocess.run([executables["menu"]], cwd="src-tauri").returncode,
]
sys.exit(next((status if status > 0 else 1 for status in statuses if status), 0))
PY
) || test_status=$?

rustup run "$toolchain" cargo llvm-cov report --manifest-path src-tauri/Cargo.toml \
  --lcov --output-path "$report_dir/lcov.info"
rustup run "$toolchain" cargo llvm-cov report --manifest-path src-tauri/Cargo.toml \
  --json --summary-only --output-path "$report_dir/summary.json"

diff-cover "$report_dir/lcov.info" --compare-branch="${COVERAGE_BASE:-origin/main}" \
  --include 'src-tauri/src/**' --fail-under=100 \
  --format "json:$report_dir/diff-coverage.json"

exit "$test_status"
