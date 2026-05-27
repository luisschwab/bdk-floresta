#!/usr/bin/env bash

# SPDX-License-Identifier: MIT OR Apache-2.0

# Checks whether any entries in `.cargo/audit.toml` can be pruned
# by running `cargo audit` against all lockfiles with ignores disabled.

set -uo pipefail

AUDIT_TOML=".cargo/audit.toml"
LOCKFILES=("Cargo.lock" "Cargo-recent.lock" "Cargo-minimal.lock")

command -v cargo-audit >/dev/null 2>&1 || { echo "cargo-audit was not found on \$PATH"; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq was not found on \$PATH"; exit 1; }

[ -f "$AUDIT_TOML" ] || { echo "> $AUDIT_TOML not found"; exit 1; }

# Look up an advisory's package name and title from the local advisory database
get_advisory_info() {
    local advisory_id="$1"

    local advisory_file
    advisory_file=$(find "${HOME}/.cargo/advisory-db" -name "${advisory_id}.md" -type f 2>/dev/null | head -1)

    [ -z "$advisory_file" ] && echo "(unknown) | (unknown)" && return

    local package title
    package=$(grep '^package = ' "$advisory_file" | sed 's/^package = "\(.*\)"$/\1/')
    title=$(grep '^# ' "$advisory_file" | head -1 | sed 's/^# //')

    echo "${package} | ${title}"
}

# Extract all ignored advisory IDs from `.cargo/audit.toml`
mapfile -t IGNORED_IDS < <(grep -v '^\s*#' "$AUDIT_TOML" | grep -oE 'RUSTSEC-[0-9]{4}-[0-9]{4}')

if [ "${#IGNORED_IDS[@]}" -eq 0 ]; then
    echo "> No ignored advisories found in $AUDIT_TOML"
    exit 0
fi

echo "> Regenerating Cargo.lock"
cargo generate-lockfile -q

# cargo-audit always reads `.cargo/audit.toml` when present
# and has no flag to bypass it, so run from a temporary directory
# (no audit.toml) while pointing --file at each lockfile
REPO_DIR="$(pwd)"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# Collect all active advisory IDs across all lockfiles
ACTIVE_ADVISORIES_IDS=""
for lockfile in "${LOCKFILES[@]}"; do
    ids=$(cd "$TEMP_DIR" && cargo audit --file "${REPO_DIR}/${lockfile}" --json 2>/dev/null | jq -r '
        [.warnings | to_entries[] | .value[].advisory.id],
        [.vulnerabilities.list[].advisory.id]
        | .[]' 2>/dev/null) || true

    ACTIVE_ADVISORIES_IDS="${ACTIVE_ADVISORIES_IDS}"$'\n'"${ids}"
done

# Partition ignored advisories into prunable and active
PRUNABLE_ADVISORIES=()

for advisory_id in "${IGNORED_IDS[@]}"; do
    if ! echo "$ACTIVE_ADVISORIES_IDS" | grep -qF "$advisory_id"; then
        PRUNABLE_ADVISORIES+=("$advisory_id")
    fi
done

echo ""

ignored_total=${#IGNORED_IDS[@]}
prunable_total=${#PRUNABLE_ADVISORIES[@]}

if [ "$prunable_total" -gt 0 ]; then
    echo "> Advisory Prune Status: FAILURE"
else
    echo "> Advisory Prune Status: SUCCESS"
fi

echo "  └── PRUNABLE: ${prunable_total}/${ignored_total}"
for ((i=0; i<prunable_total; i++)); do
    advisory_id="${PRUNABLE_ADVISORIES[$i]}"
    advisory_info=$(get_advisory_info "$advisory_id")
    if [ "$i" -eq $((prunable_total - 1)) ]; then
        echo "      └── ${advisory_id} | ${advisory_info}"
    else
        echo "      ├── ${advisory_id} | ${advisory_info}"
    fi
done

if [ "$prunable_total" -gt 0 ]; then
    exit 1
fi
