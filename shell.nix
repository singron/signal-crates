let
  sources = import ./nix/sources.nix;
  pkgs = import <nixpkgs> {
    overlays = [ (import sources.rust-overlay) ];
  };
  lib = pkgs.lib;
  stableRust = stdenvPkgs: stdenvPkgs.rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" "rust-analyzer" ];
  };
  muslRust = stdenvPkgs: stdenvPkgs.buildPackages.rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" "rust-analyzer" ];
    targets = [ "x86_64-unknown-linux-musl" ];
  };
  miriRust = stdenvPkgs: stdenvPkgs.rust-bin.nightly.latest.default.override {
    extensions = [ "rust-src" "miri" "rust-analyzer" ];
  };
  mkRustShell = stdenvPkgs: mkRust: args:
    stdenvPkgs.mkShell ({
      packages = [ (mkRust stdenvPkgs) ]
        # Avoid downloading packages from pkgsStatic if we can
        ++ (with pkgs; [ niv gdb tlaplus asciidoctor git ]);
      passthru = {
        inherit stdShell staticShell miriShell freebsdShell;
      };
    } // args);
  stdShell = mkRustShell pkgs stableRust {};
  staticShell = mkRustShell pkgs.pkgsStatic muslRust {
    CARGO_BUILD_TARGET="x86_64-unknown-linux-musl";
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${pkgs.pkgsStatic.stdenv.cc.targetPrefix}cc";
  };
  miriShell = mkRustShell pkgs miriRust {};

  mkCrossShell = target: system:
  let
    pkgsCross = import pkgs.path {
      crossSystem = { config = system; };
      overlays = [
        (import sources.rust-overlay)
        (self: super: {
          myrust = self.rust-bin.stable.latest.default.override {
            extensions = ["rust-src" "rust-analyzer"];
          };
        })
      ];
    };
  in
    pkgsCross.callPackage({
      mkShell, stdenv, myrust
    }: mkShell {
      nativeBuildInputs = [
        myrust
      ];
      "CARGO_TARGET_${(lib.strings.toUpper (lib.strings.replaceString "-" "_" target))}_LINKER" = "${stdenv.cc.targetPrefix}cc";
      CARGO_BUILD_TARGET=target;
    }) {};

  freebsdShell = mkCrossShell "x86_64-unknown-freebsd" "x86_64-unknown-freebsd";
in
  stdShell
