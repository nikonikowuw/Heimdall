# Algorithm package workspaces

`algo-packages/` contains the independent Cargo workspaces for Heimdall dynamic algorithm plugins. The workspace boundary follows the platform/runtime toolchain:

- `macos/Cargo.toml`: Apple Silicon CoreML packages.
- `rknn/rk3568/Cargo.toml`: RK3568 RKNN packages.
- `rknn/rk3576/Cargo.toml`: RK3576 RKNN packages.
- `rknn/rk3588/Cargo.toml`: RK3588 RKNN packages.

Each platform workspace owns its package members, algorithm-side dependency versions, `Cargo.lock`, and `target/` directory. The packages share the `crates/algo-sdk` source through a path dependency, but they are not members of the host workspace or of another platform workspace.

## Commands from the repository root

Run checks against the platform that matches the target toolchain:

```bash
cargo fmt --manifest-path algo-packages/macos/Cargo.toml --all
cargo check --manifest-path algo-packages/macos/Cargo.toml --workspace
cargo clippy --manifest-path algo-packages/macos/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path algo-packages/macos/Cargo.toml --workspace

cargo check --manifest-path algo-packages/rknn/rk3568/Cargo.toml --workspace
cargo check --manifest-path algo-packages/rknn/rk3576/Cargo.toml --workspace
```

The root Makefile provides the same split explicitly:

```bash
make algo-check ALGO_PLATFORM=macos
make algo-check ALGO_PLATFORM=rknn/rk3568
make algo-check ALGO_PLATFORM=rknn/rk3576
make algo-check ALGO_PLATFORM=rknn/rk3588
make algo-check-all
```

## Quick Start: Creating a New Package

Scaffold a complete, compliant algorithm package with one command:

```bash
make algo-new PLATFORM=rk3568 PKG=smoke_detection ALARM=ALARM_SMOKE
```

This generates `Cargo.toml`, `manifest.json`, `config.schema.json`, `.env`, `src/config.rs` (using `algo_config!`), `src/plugin.rs` (using unified `algo_sdk::rknn`), `src/lib.rs` (`export_algo!`), and `src/bin/run_local.rs` (`LocalPluginRunner`), and automatically registers the package in the target platform workspace.

Build one package with its package Makefile, or pass the matching platform manifest explicitly:

```bash
make -C algo-packages/macos/arm64/general_detection build
cargo build --manifest-path algo-packages/macos/Cargo.toml -p general-detection --release
```

The Makefiles copy the resulting `.so` or `.dylib` into the package-local `lib/` directory. `make package` then creates the runtime archive containing the manifest, models, test image, documentation, and dynamic library.

The host application only discovers and loads the packaged dynamic library at runtime through the manifest and C ABI. Running `cargo check --workspace` or `cargo test --workspace` at the repository root intentionally does not build these algorithm packages.
