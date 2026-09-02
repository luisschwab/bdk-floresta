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

stable := `cargo rbmt toolchains --stable`
export RBMT_LOG_LEVEL := env("RBMT_LOG_LEVEL", "progress")

_default:
    @echo "> bdk-floresta"
    @echo "> A Floresta-powered chain-source crate for BDK\n"
    @just --list

[doc: "Audit dependencies"]
audit:
    @echo "Auditing Cargo.lock"
    cargo generate-lockfile
    cargo audit --file Cargo.lock

    @echo "\nAuditing Cargo-maximum.lock"
    cargo audit --file Cargo-maximum.lock

    @echo "\nAuditing Cargo-recent.lock"
    cargo audit --file Cargo-recent.lock

    @echo "\nAuditing Cargo-minimal.lock"
    cargo audit --file Cargo-minimal.lock

[doc: "Assert Commit Bisectability"]
bisectability baseline="master":
    cargo rbmt run --baseline "{{ baseline }}" -- build --quiet

[doc: "Generate Code Coverage"]
[env("CARGO_LLVM_COV_SETUP", "yes")]
coverage:
    cargo +{{ stable }} llvm-cov \
        --all-features \
        --html \
        --ignore-filename-regex '(^|/)test[.]rs$'
    cargo +{{ stable }} llvm-cov report \
        --lcov \
        --output-path target/llvm-cov/lcov.info \
        --ignore-filename-regex '(^|/)test[.]rs$'

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
[env("BLOCKS", "25")]
[env("RBMT_LOG_LEVEL", "verbose")]
example-client-regtest:
    rm -rf examples/data/client_regtest
    cargo rbmt run -- run --release --example client_regtest

[doc: "Run the Signet Client Example"]
[env("RBMT_LOG_LEVEL", "verbose")]
example-client-signet:
    rm -rf examples/data/client_signet
    cargo rbmt run -- run --release --example client_signet

[doc: "Format Code"]
fmt:
    cargo rbmt fmt

[doc: "Regenerate Lockfiles"]
lock:
  cargo rbmt lock --lockfiles minimal,recent,maximum

[doc: "Run Tests"]
[env("RBMT_LOG_LEVEL", "verbose")]
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
    cargo rbmt tools
    cargo rbmt tools --update

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
