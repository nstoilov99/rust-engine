#!/usr/bin/env bash
# Crusty design-system lint (DESIGN.md ▸ Lint). Runs in CI and Git Bash.
#
#   scripts/lint_design.sh            # lint the tree (CI mode)
#   scripts/lint_design.sh --selftest # prove the rules: clean tree passes,
#                                     # the seeded fixtures fail
#
# Two rules:
#   1. No raw hex outside the token module (engine/.../theme/tokens.rs).
#   2. 2px is reserved (active-tab top edge + tile/node type edge); every
#      other width/thickness/stroke of 2 is a missing token.
#
# Both `rg` and `grep` exit 1 on NO match, so every check is wrapped as
# `if <search> ...; then fail`. Never `<search> ... && exit 1` under `set -e`
# — that fails the job when the check *passes*.
#
# Backend: ripgrep when available, GNU `grep -rP` otherwise (Git Bash ships
# grep but not rg). Both use the same PCRE patterns.

set -euo pipefail

cd "$(dirname "$0")/.."

# --- scope --------------------------------------------------------------
# The real tree, not the spec's `gui/ editor/ graph/ plugins/` sketch (that
# layout does not exist here). A new crate joins this list in the PR that
# creates it.
SCAN=()
for p in engine/src game_client/src crates tools; do
  [ -e "$p" ] && SCAN+=("$p")
done

# Rust only: vendored HTML/JS/CSS under crates/ is not design-system code.
#
# Excluded from BOTH rules:
#   theme/tokens.rs        — the token module IS the hex, by definition
#   transform-gizmo-fork/  — vendored third-party fork (patched for Z-up),
#                            not editor chrome; its examples/ carry upstream
#                            literals we do not own
#
# Excluded from rule 2 only (the reserved-2px allow-list):
#   graph_editor_crusty.rs — node type edge + selected-node 2px outline
#   dock_crusty.rs         — active-tab top edge
# Current status: neither actually trips the rule today — the two live 2px
# sites (asset tile edge, inspector section bar) already read
# `style.metrics.edge_accent`. The list is the *declared* carve-out, so the
# Phase-2 graph/tab work extends it here instead of inventing an exception.
SKIP_FILES_BOTH=(tokens.rs)
SKIP_DIRS=(transform-gizmo-fork target)
SKIP_FILES_PX2=(graph_editor_crusty.rs dock_crusty.rs)

# Rule 1: exactly 3/6/8 hex digits, non-hex-alphanumeric boundaries on both
# sides, comment-only lines skipped.
#   "// fixes #4021"  -> comment-only line, skipped
#   "#[derive(...)]"  -> no hex digits after `#`, no match
#   "&#x27;"          -> `&` is an excluded left boundary
HEX_RE='^(?!\s*(//|/\*|\*)).*(?<![0-9A-Za-z&])#(?:[0-9A-Fa-f]{8}|[0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})(?![0-9A-Za-z])'

# Rule 2: `width:`/`thickness:`/`stroke(` = 2 or 2.0 exactly. The trailing
# negative lookahead keeps 2.4 and 20.0 out (the published rule flagged both).
# Known gaps, stated: consts and positional constructor args
# (`Vec2::new(2.0, …)`) are not covered.
PX2_RE='(width|thickness|stroke)\s*[:=(]\s*2(?:\.0)?(?![0-9.])'

# --- search backend -----------------------------------------------------
# search <regex> <skip-file...> -- <path...>   -> 0 on match (a violation)
search() {
  local re="$1"; shift
  local skips=() paths=()
  while [ "$1" != "--" ]; do skips+=("$1"); shift; done
  shift
  paths=("$@")

  if command -v rg >/dev/null 2>&1; then
    local globs=(--glob '*.rs')
    for d in "${SKIP_DIRS[@]}"; do globs+=(--glob "!**/$d/**"); done
    for f in "${skips[@]}"; do globs+=(--glob "!**/$f"); done
    rg -nP --pcre2 "$re" "${globs[@]}" "${paths[@]}"
  else
    local opts=(--include='*.rs')
    for d in "${SKIP_DIRS[@]}"; do opts+=("--exclude-dir=$d"); done
    for f in "${skips[@]}"; do opts+=("--exclude=$f"); done
    # grep -P refuses non-UTF-8 locales; pin one.
    LC_ALL=C.UTF-8 grep -rnP "${opts[@]}" -- "$re" "${paths[@]}"
  fi
}

check_hex() {
  if search "$HEX_RE" "${SKIP_FILES_BOTH[@]}" -- "$@"; then
    echo "^^ raw hex outside theme/tokens.rs — the fix is a token, not an exception"
    return 1
  fi
  return 0
}

check_px2() {
  if search "$PX2_RE" "${SKIP_FILES_BOTH[@]}" "${SKIP_FILES_PX2[@]}" -- "$@"; then
    echo "^^ unreserved 2px width — 2px is the active-tab / type edge only"
    return 1
  fi
  return 0
}

fail=0

# --- selftest -----------------------------------------------------------
if [ "${1:-}" = "--selftest" ]; then
  FIX=scripts/lint_design_fixtures

  echo "== selftest: clean tree must pass =="
  check_hex "${SCAN[@]}" || fail=1
  check_px2 "${SCAN[@]}" || fail=1
  if [ "$fail" -ne 0 ]; then
    echo "SELFTEST FAILED: the clean tree does not pass"
    exit 1
  fi
  echo "   ok"

  echo "== selftest: seeded violations must fail =="
  for rule in hex px2; do
    if "check_$rule" "$FIX/bad_$rule.rs" >/dev/null 2>&1; then
      echo "SELFTEST FAILED: rule '$rule' did not catch $FIX/bad_$rule.rs"
      exit 1
    fi
    echo "   ok: rule '$rule' catches its fixture"
  done

  echo "== selftest: near-miss fixture must NOT trip =="
  # `// fixes #4021`, `#[derive]`, `&#x27;`, 2.4, 20.0, 24.0: all legal.
  if ! check_hex "$FIX/ok_edge_cases.rs" || ! check_px2 "$FIX/ok_edge_cases.rs"; then
    echo "SELFTEST FAILED: false positive on $FIX/ok_edge_cases.rs"
    exit 1
  fi
  echo "   ok"

  echo "lint_design selftest passed"
  exit 0
fi

# --- CI mode ------------------------------------------------------------
echo "== rule 1: no raw hex outside theme/tokens.rs =="
check_hex "${SCAN[@]}" || fail=1
echo "== rule 2: 2px is reserved =="
check_px2 "${SCAN[@]}" || fail=1

if [ "$fail" -ne 0 ]; then
  echo "lint_design: FAILED"
  exit 1
fi
echo "lint_design: OK"
