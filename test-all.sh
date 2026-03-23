#! /usr/bin/env bash

set -eu

cd -- "$( dirname -- "${BASH_SOURCE[0]}" )"

export RUST_BACKTRACE=${RUST_BACKTRACE:-1}

run() {
  local attr=$1
  local cmd=$2
  local escaped
  printf -v escaped '%q' "$cmd"
  printf 'Entering nix-shell -A %s\n' "$attr" >&2
  # Run bash in a subprocess, so that `set -x` doesn't print all the shellHook
  # stuff.
  nix-shell -A "$attr" --run "bash -x -c $escaped"
}

run stdShell 'cargo build --workspace --all-targets && RUSTFLAGS="--cfg loom" cargo build --release -p signal_lock --all-targets'
run stdShell 'cargo test --workspace --all-targets && RUSTFLAGS="--cfg loom" cargo test --release -p signal_lock'

run miriShell 'cargo miri test --workspace'
run staticShell 'cargo test --workspace --all-targets'

# Cross compile to freebsd. This roughly checks that we aren't relying on too
# many linux-isms. Ideally I would cross compile to more systems (e.g. darwin,
# illumos, netbsd), but they don't have functioning cross compilation systems
# in nixpkgs.
run freebsdShell 'cargo build --workspace --all-targets'
