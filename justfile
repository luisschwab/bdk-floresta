alias a := audit
alias c := check
alias f := fmt
alias cr := example-client-regtest
alias cs := example-client-signet
alias l := lock
alias t := test
alias sc := shellcheck
alias z := zizmor
alias p := pre-push

export RBMT_LOG_LEVEL := env("RBMT_LOG_LEVEL", "verbose")

_default:
    @echo "> bdk-floresta"
    @echo "> A Floresta-powered chain-source crate for BDK\n"
    @just --list

[doc: "Run `cargo audit` on all lockfiles and prune ignored advisories"]
audit:
    bash contrib/run-cargo-audit.sh
    bash contrib/prune-audit-ignores.sh

[doc: "Check Formatting, Linting and Documentation"]
check:
    cargo rbmt fmt --check
    cargo rbmt lint
    cargo rbmt docs

[doc: "Generate Documentation"]
docs:
    cargo rbmt docs

[doc: "Generate and Open Documentation"]
docs-open:
    cargo rbmt docs --open

[doc: "Run the Regtest Client Example"]
example-client-regtest:
    rm -rf examples/data/client_regtest
    BLOCKS=25 cargo rbmt run -- run --release --example client_regtest

[doc: "Run the Signet Client Example"]
example-client-signet:
    rm -rf examples/data/client_signet
    cargo rbmt run -- run --release --example client_signet

[doc: "Format Code"]
fmt:
    cargo rbmt fmt

[doc: "Regenerate Lockfiles"]
lock:
  cargo rbmt lock

[doc: "Run Tests"]
test:
    cargo rbmt test --toolchain stable --lockfile recent
    cargo rbmt test --toolchain stable --lockfile minimal
    cargo rbmt test --toolchain msrv --lockfile minimal

[doc: "Update Stable and Nightly Toolchains"]
toolchains:
    cargo rbmt toolchains --update-stable
    cargo rbmt toolchains --update-nightly

[doc: "Install cargo-rbmt Tools"]
tools:
    RBMT_LOG_LEVEL=progress cargo rbmt tools

[doc: "Run ShellCheck"]
shellcheck:
    @command -v shellcheck >/dev/null 2>&1 || { echo "shellcheck was not found on \$PATH" && exit 1; }
    find . -name '*.sh' -print -exec shellcheck {} +

[doc: "Run Zizmor"]
zizmor:
    zizmor .

[doc: "Run pre-push checks"]
pre-push:
    @just lock
    @just check
    @just docs
    @just test
    @just shellcheck
    @just zizmor
    @just audit
