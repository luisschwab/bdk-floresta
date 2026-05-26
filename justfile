alias a := audit
alias b := build
alias c := check
alias d := delete
alias f := fmt
alias cr := example-client-regtest
alias cs := example-client-signet
alias l := lock
alias t := test
alias sc := shellcheck
alias z := zizmor
alias p := pre-push

_default:
    @echo "> bdk-floresta"
    @echo "> A Floresta-powered chain-source crate for BDK\n"
    @just --list

[doc: "Run `cargo audit` on all lockfiles and prune ignored advisories"]
audit:
    bash contrib/run-cargo-audit.sh
    bash contrib/prune-audit-ignores.sh

[doc: "Build `bdk-floresta` and examples"]
build:
    RBMT_LOG_LEVEL=verbose cargo rbmt build --release
    RBMT_LOG_LEVEL=verbose cargo rbmt build --release --examples

[doc: "Check code formatting, compilation, linting"]
check:
    RBMT_LOG_LEVEL=verbose cargo rbmt fmt --check
    RBMT_LOG_LEVEL=verbose cargo rbmt lint
    RBMT_LOG_LEVEL=verbose cargo rbmt docsrs

[doc: "Check that all feature combinations compile"]
check-features:
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain stable --lock-file recent

[doc: "Check if commits are PGP-signed"]
check-commit-signatures:
    bash contrib/check-commit-signatures.sh

[doc: "Delete files: data, target, lockfiles"]
delete item="data":
    just _delete-{{ item }}

[doc: "Generate documentation"]
doc:
    RBMT_LOG_LEVEL=verbose cargo rbmt docsrs

[doc: "Generate and open documentation"]
doc-open:
    RBMT_LOG_LEVEL=verbose cargo rbmt docsrs --open

[doc: "Run the `client_regtest` example"]
example-client-regtest:
    rm -rf examples/data/client_regtest
    cargo run --release --example client_regtest

[doc: "Run the `client_signet` example"]
example-client-signet:
    #rm -rf examples/data/client_signet
    RBMT_LOG_LEVEL=progress cargo rbmt run --release --example client_signet

[doc: "Format code"]
fmt:
    RBMT_LOG_LEVEL=verbose cargo rbmt fmt

[doc: "Regenerate `Cargo-recent.lock` and `Cargo-minimal.lock`"]
lock:
  RBMT_LOG_LEVEL=verbose cargo rbmt lock

[doc: "Run tests across all toolchains and lockfiles"]
test:
    RBMT_LOG_LEVEL=verbose cargo rbmt test

[doc: "Install and/or Update `cargo-rbmt` and Stable and Nightly toolchains"]
toolchains:
    bash contrib/install-cargo-rbmt.sh
    RBMT_LOG_LEVEL=verbose cargo rbmt toolchains --update-stable
    RBMT_LOG_LEVEL=verbose cargo rbmt toolchains --update-nightly

[doc: "Run ShellCheck"]
shellcheck:
    @command -v shellcheck >/dev/null 2>&1 || { echo "shellcheck was not found on \$PATH" && exit 1; }
    find . -name '*.sh' -print -exec shellcheck {} +

[doc: "Run Zizmor"]
zizmor:
    uvx zizmor .

[doc: "Run pre-push checks"]
pre-push:
    @just lock
    @just check
    @just doc
    @just test
    @just shellcheck
    @just zizmor
    @just audit
    @just check-commit-signatures
    @just example-client-regtest

_delete-data:
    rm -rf examples/data

_delete-target:
    rm -rf target/

_delete-lockfiles:
    rm -f Cargo.lock
    rm -f Cargo-recent.lock
    rm -f Cargo-minimal.lock
