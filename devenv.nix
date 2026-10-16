{ pkgs, ... }:

let
  rust-glancer = pkgs.callPackage ./nix/rust-glancer.nix { };
in
{
  name = "piton";

  languages.rust = {
    enable = true;
    toolchainFile = ./rust-toolchain.toml;
  };

  packages = with pkgs; [
    just
    cargo-nextest
    cargo-deny
    cargo-machete
    rust-glancer
  ];

  env.CARGO_TERM_COLOR = "always";

  git-hooks.hooks = {
    rustfmt = {
      enable = true;
      settings.check = true;
    };

    # Clippy runs through `just clippy`, so the hook, local runs and CI lint
    # the same way.
    clippy = {
      enable = true;
      name = "clippy";
      entry = "${pkgs.just}/bin/just clippy";
      files = "\\.rs$|Cargo\\.(toml|lock)$|clippy\\.toml$";
      pass_filenames = false;
    };
  };

  tasks."piton:ci" = {
    exec = "just ci";
    before = [ "devenv:enterTest" ];
  };
}
