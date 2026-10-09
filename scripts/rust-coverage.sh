#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Darwin) ;;
  *) echo "The Rust coverage pilot requires macOS" >&2; exit 1 ;;
esac

toolchain=1.93.1
test_filter=commands::claude_config::tests::
report_dir=coverage/rust/macos
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/worktreemanager-target}"
export CARGO_LLVM_COV_TARGET_DIR="$CARGO_TARGET_DIR/llvm-cov-pilot"

rm -rf "$report_dir"
version=$(rustup run "$toolchain" cargo llvm-cov --version)
if [ "$version" != 'cargo-llvm-cov 0.9.1' ]; then
  echo "Expected cargo-llvm-cov 0.9.1, got $version" >&2
  exit 1
fi

mkdir -p "$report_dir"
rustup run "$toolchain" rustc -Vv > "$report_dir/toolchain.txt"
echo "$version" >> "$report_dir/toolchain.txt"
echo "$test_filter" >> "$report_dir/toolchain.txt"

/usr/bin/time -p -o "$report_dir/baseline-build.txt" \
  rustup run "$toolchain" cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --no-run
/usr/bin/time -p -o "$report_dir/baseline-test.txt" \
  rustup run "$toolchain" cargo test --locked --manifest-path src-tauri/Cargo.toml --lib "$test_filter"
rustup run "$toolchain" cargo llvm-cov clean --workspace --manifest-path src-tauri/Cargo.toml
coverage_env=$(CARGO_TARGET_DIR="$CARGO_LLVM_COV_TARGET_DIR" \
  rustup run "$toolchain" cargo llvm-cov show-env --sh --manifest-path src-tauri/Cargo.toml)
test_status=0
(
  eval "$coverage_env"
  export CARGO_TARGET_DIR="$CARGO_LLVM_COV_TARGET_DIR"
  /usr/bin/time -p -o "$report_dir/instrumented-build.txt" \
    rustup run "$toolchain" cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --no-run || exit $?
  /usr/bin/time -p -o "$report_dir/instrumented-test.txt" \
    rustup run "$toolchain" cargo test --locked --manifest-path src-tauri/Cargo.toml --lib "$test_filter"
) || test_status=$?

rustup run "$toolchain" cargo llvm-cov report --manifest-path src-tauri/Cargo.toml \
  --lcov --output-path "$report_dir/lcov.info"
rustup run "$toolchain" cargo llvm-cov report --manifest-path src-tauri/Cargo.toml \
  --json --summary-only --output-path "$report_dir/summary.json"

exit "$test_status"
