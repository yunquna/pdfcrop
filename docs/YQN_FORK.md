# YQN pdfcrop fork

- Upstream: <https://github.com/pdfcrop/pdfcrop>
- YQN release line: `v0.1.1-yqn.N`
- Verified Rust baseline: `1.88.0` with `wasm32-unknown-unknown`.
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

The upstream manifest declared Rust 1.86, but Hayro 0.4 uses language/library
features unavailable to 1.86. YQN verified native and WASM builds on Rust 1.88
and records that version as the actual fork baseline instead of downgrading or
patching Hayro.

WASM builds disable the default Rayon feature:

```bash
PATH="$(dirname "$(rustup which --toolchain 1.88.0 rustc)"):$PATH" \
  wasm-pack build --target nodejs --out-dir pkg-node --release --no-opt -- \
  --no-default-features --features std
node tests/node-smoke.cjs
PATH="$(dirname "$(rustup which --toolchain 1.88.0 rustc)"):$PATH" \
  wasm-pack build --target bundler --out-dir pkg-worker --release --no-opt -- \
  --no-default-features --features std
node scripts/prepare-yqn-package.mjs pkg-worker
```

NodeJS output is smoke-test only. The GitHub Release artifact is the bundler
package `@yunquna/pdfcrop-wasm`. This fork does not publish to npm in V1.

## Local build evidence

- Rust `1.88.0`, `wasm-pack 0.15.0`, `wasm-opt 132`;
- Node smoke: passed with one synthetic in-memory PDF;
- bundler WASM: 3,818,642 bytes; gzip level 9: 1,536,413 bytes;
- local npm tarball: 1,544,730 bytes;
- release URL/source commit/release SHA-256 remain pending until the feature PR is
  reviewed, merged and tagged. Local tarball hashes are not release evidence.

## Release attempts

- `v0.1.1-yqn.1` points to the first stable YQN main baseline. Its workflow
  stopped before build because crates.io installation of `wasm-pack 0.15.0`
  resolved a tool-only dependency requiring Rust 1.91. No GitHub Release or
  artifact was created.
- `v0.1.1-yqn.2` keeps the project on Rust 1.88 and installs the official
  precompiled `wasm-pack 0.15.0` binary through `taiki-e/install-action`; this
  avoids compiling build tooling with the project toolchain.
- `v0.1.1-yqn.2` then reached Node smoke but Ubuntu's older `wasm-opt` produced
  an invalid externref table for the Node-only smoke package. No Release or
  artifact was created.
- `v0.1.1-yqn.3` disables wasm-pack's automatic optimization for both generated
  packages. Node smoke remains unoptimized; the Worker package receives exactly
  one explicit `wasm-opt -Oz` pass before packing.
