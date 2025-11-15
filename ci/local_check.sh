#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   LOCAL_CHECK_ONLINE=0 LOCAL_CHECK_STRICT=1 LOCAL_CHECK_VERBOSE=1 ci/local_check.sh
# Defaults: online, non-strict, minimal logging.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ONLINE="${LOCAL_CHECK_ONLINE:-1}"
STRICT="${LOCAL_CHECK_STRICT:-0}"
VERBOSE="${LOCAL_CHECK_VERBOSE:-0}"

if [[ "$VERBOSE" != "0" ]]; then
  set -x
fi

SKIP_CODE=97
SKIPPED_STEPS=()
LAST_SKIP_REASON=""

have() {
  command -v "$1" >/dev/null 2>&1
}

need() {
  if have "$1"; then
    return 0
  fi
  echo "[miss] $1" >&2
  return 1
}

step() {
  echo ""
  echo "▶ $*"
}

skip_step() {
  LAST_SKIP_REASON="$1"
  echo "   ↳ $1"
  return "$SKIP_CODE"
}

ensure_tool() {
  local tool="$1"
  if need "$tool"; then
    return 0
  fi

  if [[ "$STRICT" == "1" ]]; then
    echo "[fail] Required tool missing in STRICT mode: $tool" >&2
    return 1
  fi

  skip_step "missing tool: $tool"
}

require_file() {
  local path="$1"
  if [[ -f "$path" ]]; then
    return 0
  fi

  if [[ "$STRICT" == "1" ]]; then
    echo "[fail] Required file missing in STRICT mode: $path" >&2
    return 1
  fi

  skip_step "missing file: $path"
}

require_online() {
  local desc="$1"
  if [[ "$ONLINE" == "1" ]]; then
    return 0
  fi

  if [[ "$STRICT" == "1" ]]; then
    echo "[warn] STRICT requested but '$desc' needs network; set LOCAL_CHECK_ONLINE=1" >&2
  fi

  skip_step "'$desc' requires LOCAL_CHECK_ONLINE=1"
}

run_or_skip() {
  local desc="$1"
  shift

  LAST_SKIP_REASON=""
  step "$desc"
  local status=0
  if "$@"; then
    return 0
  else
    status=$?
  fi

  if [[ "$status" -eq "$SKIP_CODE" ]]; then
    local summary="$desc"
    if [[ -n "$LAST_SKIP_REASON" ]]; then
      summary="$summary - $LAST_SKIP_REASON"
      echo "[skip] $desc ($LAST_SKIP_REASON)"
    else
      echo "[skip] $desc"
    fi
    SKIPPED_STEPS+=("$summary")
    return 0
  fi

  echo "[fail] $desc (exit $status)" >&2
  exit "$status"
}

print_env() {
  echo "LOCAL_CHECK_ONLINE=${ONLINE}"
  echo "LOCAL_CHECK_STRICT=${STRICT}"
  echo "LOCAL_CHECK_VERBOSE=${VERBOSE}"
}

print_versions() {
  local tools=(rustc cargo rustfmt clippy-driver jq git)
  for tool in "${tools[@]}"; do
    if have "$tool"; then
      "$tool" --version || true
    else
      echo "[miss] $tool"
    fi
  done
}

autotag_probe() {
  local script_path="$ROOT/scripts/version-tools.sh"
  require_file "$script_path" || return $?
  ensure_tool bash || return $?
  ensure_tool cargo || return $?
  ensure_tool jq || return $?
  ensure_tool git || return $?

  bash -c 'set -euo pipefail; source "$1"; list_crates >/dev/null' _ "$script_path"
}

cargo_fmt_check() {
  ensure_tool cargo || return $?
  cargo fmt --all -- --check
}

cargo_clippy_check() {
  ensure_tool cargo || return $?
  cargo clippy --workspace --all-targets -- -D warnings
}

cargo_build_check() {
  ensure_tool cargo || return $?
  cargo build --workspace --all-features
}

cargo_test_check() {
  ensure_tool cargo || return $?
  cargo test --workspace --all-features -- --nocapture
}

publish_parity() {
  require_online "publish parity" || return $?
  local script_path="$ROOT/scripts/version-tools.sh"
  require_file "$script_path" || return $?
  ensure_tool cargo || return $?
  ensure_tool jq || return $?

  local crates=()
  while IFS= read -r entry; do
    crates+=("$entry")
  done < <(bash -c 'set -euo pipefail; source "$1"; list_crates' _ "$script_path")

  if [[ "${#crates[@]}" -eq 0 ]]; then
    skip_step "no crates detected"
  fi

  local is_workspace=0
  if grep -q "^\[workspace\]" "$ROOT/Cargo.toml"; then
    is_workspace=1
  fi

  for entry in "${crates[@]}"; do
    local name
    name="$(awk '{print $1}' <<<"$entry")"
    if [[ "$is_workspace" -eq 1 ]]; then
      cargo package -p "$name" --allow-dirty >/dev/null
    else
      cargo package --allow-dirty >/dev/null
      break
    fi
  done

  if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
    for entry in "${crates[@]}"; do
      local name
      name="$(awk '{print $1}' <<<"$entry")"
      if [[ "$is_workspace" -eq 1 ]]; then
        cargo publish --dry-run -p "$name" --allow-dirty >/dev/null
      else
        cargo publish --dry-run --allow-dirty >/dev/null
        break
      fi
    done
  else
    echo "   ↳ CARGO_REGISTRY_TOKEN not set; publish dry-run skipped."
  fi
}

echo "== Greentic Session :: local CI check =="
print_env
step "Toolchain versions"
print_versions

run_or_skip "Auto-tag helper scripts" autotag_probe
run_or_skip "cargo fmt --all -- --check" cargo_fmt_check
run_or_skip "cargo clippy --workspace --all-targets" cargo_clippy_check
run_or_skip "cargo build --workspace --all-features" cargo_build_check
run_or_skip "cargo test --workspace --all-features -- --nocapture" cargo_test_check
run_or_skip "Publish parity (package + optional dry-run)" publish_parity

echo ""
if [[ "${#SKIPPED_STEPS[@]}" -gt 0 ]]; then
  echo "Skipped steps:"
  for entry in "${SKIPPED_STEPS[@]}"; do
    echo " - $entry"
  done
fi
echo "Local checks complete."
