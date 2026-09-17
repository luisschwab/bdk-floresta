alias a := audit
alias b := bisectability
alias c := check
alias cov := coverage
alias d := docs
alias do := docs-open
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

# Quality

[doc: "Audit Cargo Dependencies"]
[group("Quality")]
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
[group("Quality")]
bisectability baseline="master":
    cargo rbmt run --baseline "{{ baseline }}" -- build --quiet

[doc: "Check Formatting, Linting and Documentation"]
[group("Quality")]
check:
    cargo rbmt fmt --check
    cargo rbmt lint
    cargo rbmt docs

[doc: "Format Code"]
[group("Quality")]
fmt:
    cargo rbmt fmt

[doc: "Run Pre-Push Checks"]
[group("Quality")]
pre-push:
    # Generate Lockfiles
    cargo rbmt lock --lockfiles minimal,recent,maximum
    # Check Formatting
    cargo rbmt fmt --check
    # Check Linting
    cargo rbmt lint
    # Check Documentation
    cargo rbmt docs
    # Check PR Bisectability
    cargo rbmt run --baseline master -- test --quiet
    # Run Tests
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain stable --lockfile recent
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain stable --lockfile minimal
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain msrv --lockfile minimal
    # Audit Cargo Dependencies
    @just audit
    # Audit Shell Scripts
    @just shellcheck
    # Audit CI Files
    @just zizmor

[doc: "Run ShellCheck"]
[group("Quality")]
shellcheck:
    @command -v shellcheck >/dev/null 2>&1 || { echo "shellcheck was not found on \$PATH" && exit 1; }
    find . -name '*.sh' -print -exec shellcheck {} +

[doc: "Run Zizmor"]
[group("Quality")]
zizmor:
    zizmor .

# Documentation

[doc: "Generate Documentation"]
[group("Documentation")]
docs:
    cargo rbmt docs

[doc: "Generate and Open Documentation"]
[group("Documentation")]
docs-open:
    cargo rbmt docs --open

# Testing

[doc: "Generate Coverage Report"]
[env("CARGO_LLVM_COV_SETUP", "yes")]
[group("Testing")]
coverage:
    cargo +{{ stable }} llvm-cov \
        --all-features \
        --html \
        --ignore-filename-regex '(^|/)test[.]rs$'
    cargo +{{ stable }} llvm-cov report \
        --lcov \
        --output-path target/llvm-cov/lcov.info \
        --ignore-filename-regex '(^|/)test[.]rs$'

[doc: "Run Tests"]
[group("Testing")]
test:
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain stable --lockfile recent
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain stable --lockfile minimal
    RBMT_LOG_LEVEL=verbose cargo rbmt test --toolchain msrv --lockfile minimal

# Examples

[doc: "Run the Regtest Client Example"]
[env("BLOCKS", "25")]
[env("RBMT_LOG_LEVEL", "verbose")]
[group("Examples")]
example-client-regtest:
    rm -rf examples/data/client_regtest
    cargo rbmt run -- run --release --example client_regtest

[doc: "Run the Signet Client Example"]
[env("RBMT_LOG_LEVEL", "verbose")]
[group("Examples")]
example-client-signet:
    rm -rf examples/data/client_signet
    cargo rbmt run -- run --release --example client_signet

# Dependencies

[doc: "Regenerate Lockfiles"]
[group("Dependencies")]
lock:
  cargo rbmt lock --lockfiles minimal,recent,maximum

# Setup

[doc: "Install Tools and Toolchains"]
[group("Setup")]
install-tools-toolchains:
    cargo rbmt tools
    cargo rbmt toolchains

[doc: "Update Tools and Toolchains"]
[group("Setup")]
update-tools-toolchains:
    cargo rbmt tools --update
    cargo rbmt toolchains --update-stable
    cargo rbmt toolchains --update-nightly
