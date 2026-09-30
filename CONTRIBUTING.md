# Contributing to DriftFS

Thank you for your interest in contributing. This document covers what you need to know to submit changes.

## Getting Started

1. Fork and clone the repository.
2. Install prerequisites listed in [README.md](README.md#prerequisites).
3. Build the workspace: `cargo build --all-targets`.
4. Run the test suite: `cargo test --all`.

## Making Changes

### Branch Conventions

- Create feature branches from `main`.
- Use descriptive branch names: `fix/cache-eviction-panic`, `feat/linux-fuse-adapter`.

### Code Standards

- **Formatting**: Run `cargo fmt --all` before committing.
- **Linting**: Run `cargo clippy --all-targets` and resolve all warnings.
- **Tests**: All existing tests must pass. Add tests for new behavior.
- **Comments**: Maintain clean code comments. Do not add obvious narration comments. Document non-obvious invariants and public API contracts only.

### Verification Gate

Every pull request must pass the mandatory verification gate:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets
cargo test --all
```

CI runs these checks automatically on every push and pull request.

### Commit Messages

- Use present tense: "Fix cache eviction" not "Fixed cache eviction".
- Keep the first line under 72 characters.
- Reference issue numbers when applicable.

## Pull Request Process

1. Ensure the verification gate passes locally before pushing.
2. Open a pull request against `main` with a clear description of the change.
3. Link related issues in the PR description.
4. Address review feedback with additional commits (do not force-push during review).

## Architecture & Design

Before making architectural changes, review:

- [docs/architecture.md](docs/architecture.md): system design, data flows, and security invariants.

## Reporting Bugs

Open an issue with:

1. Steps to reproduce.
2. Expected vs. actual behavior.
3. OS version and WinFsp version (if applicable).
4. Relevant log output (with secrets redacted).

## Security Vulnerabilities

See [SECURITY.md](SECURITY.md) for responsible disclosure instructions. Do not open public issues for security vulnerabilities.
