#!/bin/bash

set -x
set -euo pipefail

# Pin dependencies for MSRV (1.85.0)
cargo update time --precise 0.3.36
