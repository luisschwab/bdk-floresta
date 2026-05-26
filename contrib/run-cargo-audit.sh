#!/usr/bin/env bash

# SPDX-License-Identifier: MIT OR Apache-2.0

# This script runs `cargo audit` against all three lockfiles
# and prints a structured summary of warnings and vulnerabilities.
#
# Vulnerabilities and warnings which do not stem from direct
# dependencies show the full dependency tree for the affected package.

set -uo pipefail

# Check if `cargo-audit`, `cargo-rbmt`, `jq`, and `python3` are installed
command -v cargo-audit >/dev/null 2>&1 || { echo "cargo-audit was not found on \$PATH"; exit 1; }
command -v cargo-rbmt >/dev/null 2>&1 || { echo "cargo-rbmt was not found on \$PATH"; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq was not found on \$PATH"; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 was not found on \$PATH"; exit 1; }

WARNING_COUNTER=0
VULNERABILITY_COUNTER=0

LOCKFILES=("Cargo.lock" "Cargo-recent.lock" "Cargo-minimal.lock")

declare -A AUDIT_OUTPUT
declare -A WARNING_COUNT
declare -A VULNERABILITY_COUNT
declare -A DEPENDENCY_PATHS

# Returns the dependency path from the workspace
# root to the given package using `cargo tree -i`
get_dependency_path() {
    local package_name="$1"
    local package_version="$2"

    local tree_output
    tree_output=$(
        cargo tree -i "${package_name}@${package_version}" \
        --target all --edges normal,dev,build --prefix depth \
        --format "{p}" 2>/dev/null
    ) || return 1

    local -a path=()
    local depth_previous=-1
    while IFS= read -r line; do
        local depth
        local package
        depth=$(printf '%s' "$line" | grep -o '^[0-9]\+')
        [ -z "$depth" ] && break
        package=$(printf '%s' "$line" | sed 's/^[0-9]*//' | sed 's/ \(v[^ ]*\).*/@\1/')

        if [ "$depth" -eq $((depth_previous + 1)) ]; then
            path+=("$package")
            depth_previous=$depth
        else
            break
        fi
    done <<< "$tree_output"

    [ ${#path[@]} -eq 0 ] && return 1

    local -a reversed=()
    for ((i=${#path[@]}-1; i>=0; i--)); do
        reversed+=("${path[$i]}")
    done

    local path_formatted
    path_formatted=$(printf '%s -> ' "${reversed[@]}")
    echo "${path_formatted% -> }"
}

# Fallback for `get_dependency_path`
# BFS through `cargo metadata --locked`'s resolve
# graph, which includes optional/feature-gated dependencies
# that `cargo tree -i` skips. Used for packages that are in
# the lockfile but not in the active dependency graph.
get_dependency_path_from_metadata() {
    local package_name="$1"
    local package_version="$2"

    local metadata
    metadata=$(cargo metadata --locked --format-version 1 2>/dev/null) || return 1

    echo "$metadata" | BFS_NAME="$package_name" BFS_VERSION="$package_version" python3 -c "
import json, sys, os, collections
metadata = json.load(sys.stdin)
target_name    = os.environ['BFS_NAME']
target_version = os.environ['BFS_VERSION']
packages_by_id = {package['id']: package for package in metadata['packages']}
target_id = next((package['id'] for package in metadata['packages'] if package['name'] == target_name and package['version'] == target_version), None)
if not target_id:
    sys.exit(1)
reverse_deps = collections.defaultdict(list)
for node in metadata['resolve']['nodes']:
    for dependency in node['deps']:
        reverse_deps[dependency['pkg']].append(node['id'])
workspace_roots = set(metadata['workspace_members'])
queue   = collections.deque([[target_id]])
visited = {target_id}
while queue:
    path    = queue.popleft()
    current = path[-1]
    if current in workspace_roots:
        path_parts = [packages_by_id[package_id]['name'] + '@v' + packages_by_id[package_id]['version'] for package_id in reversed(path)]
        print(' -> '.join(path_parts))
        sys.exit(0)
    for parent in reverse_deps[current]:
        if parent not in visited:
            visited.add(parent)
            queue.append(path + [parent])
sys.exit(1)
"
}

# Format audit findings into a tree for display
format_findings() {
    local audit_output="$1"
    local type="$2"
    local indent="$3"

    local findings
    if [ "$type" = "warnings" ]; then
        findings=$(echo "$audit_output" | jq -c '.warnings | to_entries[] | .value[]')
    else
        findings=$(echo "$audit_output" | jq -c '.vulnerabilities.list[]')
    fi

    [ -z "$findings" ] && return

    local -a finding_lines=()
    while IFS= read -r finding; do
        [ -z "$finding" ] && continue
        finding_lines+=("$finding")
    done <<< "$findings"

    local total=${#finding_lines[@]}
    [ "$total" -eq 0 ] && return

    for ((index=0; index<total; index++)); do
        local finding="${finding_lines[$index]}"
        local advisory_id
        local package_name
        local package_version
        local title
        advisory_id=$(echo "$finding" | jq -r '.advisory.id')
        package_name=$(echo "$finding" | jq -r '.package.name')
        package_version=$(echo "$finding" | jq -r '.package.version')
        title=$(echo "$finding" | jq -r '.advisory.title')

        local branch
        local branch_transitive
        if [ "$index" -eq $((total - 1)) ]; then
            branch="${indent}└── "
            branch_transitive="${indent}    └── "
        else
            branch="${indent}├── "
            branch_transitive="${indent}│   └── "
        fi

        echo "${branch}$advisory_id | $package_name@v$package_version | $title"

        local dependency_path_key="${package_name}@v${package_version}"
        if [ -n "${DEPENDENCY_PATHS[$dependency_path_key]+x}" ]; then
            local dependency_path="${DEPENDENCY_PATHS[$dependency_path_key]}"
            if [[ "$dependency_path" == "__LOCKFILE_ONLY__:"* ]]; then
                echo "${branch_transitive}TRANSITIVE [LOCKFILE-ONLY]: ${dependency_path#__LOCKFILE_ONLY__:}"
            else
                echo "${branch_transitive}TRANSITIVE: ${dependency_path}"
            fi
        fi

        [ "$index" -lt $((total - 1)) ] && echo "${indent}│"
    done
}

# Copy the given lockfile to `Cargo.lock`, run `cargo audit`,
# cache transitive dependency paths, and then remove `Cargo.lock`
process_lockfile() {
    local lockfile="$1"

    if [ "$lockfile" != "Cargo.lock" ]; then
        cp "$lockfile" Cargo.lock
    fi

    local audit_output
    audit_output=$(cargo audit --json 2>/dev/null) || true

    local vulnerabilities
    vulnerabilities=$(echo "$audit_output" | jq '.vulnerabilities.count // 0')

    local warnings
    warnings=$(echo "$audit_output" | jq '[.warnings | to_entries[] | .value | length] | add // 0')

    WARNING_COUNTER=$((WARNING_COUNTER + warnings))
    VULNERABILITY_COUNTER=$((VULNERABILITY_COUNTER + vulnerabilities))

    WARNING_COUNT[$lockfile]=$warnings
    VULNERABILITY_COUNT[$lockfile]=$vulnerabilities
    AUDIT_OUTPUT[$lockfile]=$audit_output

    # Cache dependency paths for all transitive findings while `Cargo.lock` is present
    local findings_all
    findings_all=$(echo "$audit_output" | jq -r '
        [.warnings | to_entries[] | .value[] | "\(.package.name)@v\(.package.version)"],
        [.vulnerabilities.list[] | "\(.package.name)@v\(.package.version)"]
        | .[]' | sort -u)

    while IFS= read -r package_key; do
        [ -z "$package_key" ] && continue
        local package_name
        local package_version
        package_name="${package_key%@v*}"
        package_version="${package_key##*@v}"

        if echo "$DEPENDENCIES_DIRECT" | grep -qx "$package_name"; then
            continue
        fi

        if [ -z "${DEPENDENCY_PATHS[$package_key]+x}" ]; then
            local dependency_path
            dependency_path=$(get_dependency_path "$package_name" "$package_version") || true
            if [ -n "$dependency_path" ]; then
                DEPENDENCY_PATHS[$package_key]="$dependency_path"
            else
                local lockfile_only_path
                lockfile_only_path=$(get_dependency_path_from_metadata "$package_name" "$package_version") || true
                [ -n "$lockfile_only_path" ] && DEPENDENCY_PATHS[$package_key]="__LOCKFILE_ONLY__:${lockfile_only_path}"
            fi
        fi
    done <<< "$findings_all"

    rm -f Cargo.lock
}

echo "> Regenerating Cargo.lock"
cargo generate-lockfile -q

DEPENDENCIES_DIRECT=$(cargo metadata --format-version 1 2>/dev/null | jq -r '
    .workspace_members as $members |
    [.packages[] | select(.id | IN($members[])) | .dependencies[].name] | unique[]
')

echo "> Auditing Cargo.lock"
process_lockfile "Cargo.lock"

echo "> Auditing Cargo-recent.lock"
process_lockfile "Cargo-recent.lock"

echo "> Auditing Cargo-minimal.lock"
process_lockfile "Cargo-minimal.lock"

echo ""

if [ "$WARNING_COUNTER" -gt 0 ] || [ "$VULNERABILITY_COUNTER" -gt 0 ]; then
    echo "> Audit Status: FAILED"
else
    echo "> Audit Status: SUCCESS"
fi

lockfile_total=${#LOCKFILES[@]}

echo "  ├── WARNINGS: $WARNING_COUNTER"
for ((i=0; i<lockfile_total; i++)); do
    lockfile="${LOCKFILES[$i]}"
    if [ "$i" -eq $((lockfile_total - 1)) ]; then
        echo "  │   └── $lockfile: ${WARNING_COUNT[$lockfile]}"
        format_findings "${AUDIT_OUTPUT[$lockfile]}" "warnings" "  │       "
    else
        echo "  │   ├── $lockfile: ${WARNING_COUNT[$lockfile]}"
        format_findings "${AUDIT_OUTPUT[$lockfile]}" "warnings" "  │   │   "
    fi
done

echo "  └── VULNERABILITIES: $VULNERABILITY_COUNTER"
for ((i=0; i<lockfile_total; i++)); do
    lockfile="${LOCKFILES[$i]}"
    if [ "$i" -eq $((lockfile_total - 1)) ]; then
        echo "      └── $lockfile: ${VULNERABILITY_COUNT[$lockfile]}"
        format_findings "${AUDIT_OUTPUT[$lockfile]}" "vulnerabilities" "          "
    else
        echo "      ├── $lockfile: ${VULNERABILITY_COUNT[$lockfile]}"
        format_findings "${AUDIT_OUTPUT[$lockfile]}" "vulnerabilities" "      │   "
    fi
done

if [ "$WARNING_COUNTER" -gt 0 ] || [ "$VULNERABILITY_COUNTER" -gt 0 ]; then
    exit 1
fi
