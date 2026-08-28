# YQN pdfcrop fork

- Upstream: <https://github.com/pdfcrop/pdfcrop>
- YQN release line: `v0.1.1-yqn.N`
- `main` is the stable YQN release baseline; `dev` is integration.
- YQN-only patches are limited to page-box policy, target-page normalization,
  deadline checks, stable WASM results, and Worker packaging.
- Generic fixes should be proposed upstream before long-term divergence.
- Customer PDFs, labels, object URLs, credentials, and generated runtime files
  must never enter this repository.

## Upstream baseline

The untouched upstream `0.1.1` baseline passes all 22 unit tests and 2 doc tests.
It does not currently pass repository-wide `cargo fmt --check` or
`cargo clippy --workspace --all-targets -- -D warnings` because of pre-existing
format drift and warnings outside the YQN patch scope.

YQN changes must not introduce new warnings. Files changed by YQN must pass
`rustfmt`; focused tests for changed behavior and the full upstream test suite
must pass before a YQN release. The fork does not bulk-format or opportunistically
repair unrelated upstream files.
