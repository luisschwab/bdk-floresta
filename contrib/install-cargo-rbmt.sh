#!/usr/bin/env bash

# SPDX-License-Identifier: MIT OR Apache-2.0

# This script will install/update `cargo-rbmt`
# from crates.io using the version specified in
# the `rbmt-version` file.
#
# The version may be specified as:
#  - cargo-rbmt-0.3.0
#  - cargo-rbmt-v0.3.0
#  - v0.3.0
#  - 0.3.0

set -euo pipefail

RBMT_VERSION=$(cat rbmt-version)
RBMT_VERSION="${RBMT_VERSION#cargo-rbmt-}"
RBMT_VERSION="${RBMT_VERSION#v}"

echo "> Installing cargo-rbmt @ v$RBMT_VERSION"
cargo install cargo-rbmt --version "$RBMT_VERSION" --locked
