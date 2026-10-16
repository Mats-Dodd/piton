# piton

Prefer straightforward designs and delete accidental complexity rather than preserving it.

This repository is a Rust workspace. Each crate under `crates/` sits on a real architectural boundary. Add a crate only when the code has a new boundary of this kind; do not create speculative crates. Every crate must inherit workspace package metadata and lint configuration.

Every external dependency is declared once in the root `[workspace.dependencies]` with default features off; crates opt into exactly the features they use.

Lints are strict (`Cargo.toml` `[workspace.lints]` and `clippy.toml`): hash-ordered collections, unbounded channels, `debug_assert!` and vague names are banned, and nothing is printed with `println!`. When an exception is genuinely needed, use `#[expect(..., reason = "...")]`, never `#[allow]`.

Run `just ci` before committing changes. Keep local and CI validation on the same path.

The toolchain and tools come from devenv (`devenv shell`). The development environment uses Rust Glancer rather than rust-analyzer. Glancer requires the `rust-src` toolchain component and is provided by devenv as `rust-glancer`.
