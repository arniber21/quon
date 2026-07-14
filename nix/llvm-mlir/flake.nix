# Unified LLVM + MLIR 22 prefix for Melior / mlir-sys.
#
# Nix splits llvmPackages_22.llvm and llvmPackages_22.mlir into separate store
# paths. mlir-sys expects a single prefix with:
#   bin/llvm-config, lib/libMLIR*.a, include/mlir-c/
#
# Stock llvm-config hardcodes the LLVM-only store path for --prefix/--libdir/
# --includedir, so this flake wraps it to report the joined prefix.
#
# Pin matches Nixhub llvmPackages_22.mlir@22.1.8.
{
  description = "Melior-compatible LLVM+MLIR 22 toolchain prefix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/3d46470bb3030020f7e1361f33514854f5bfa86d";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          llvmPkgs = pkgs.llvmPackages_22;
          # Prefer `llvm` (tools + libs) over bare `libllvm` so `llvm-config`
          # and FileCheck land in the joined prefix.
          llvm-mlir = pkgs.symlinkJoin {
            name = "llvm-mlir-22";
            paths = [
              llvmPkgs.llvm
              llvmPkgs.llvm.lib
              llvmPkgs.llvm.dev
              llvmPkgs.mlir
              llvmPkgs.mlir.dev
              # bindgen (z3-sys / mlir-sys) needs libclang at link/discover time
              llvmPkgs.libclang.lib
            ];
            # Multiple LLVM outputs can collide on shared cmake/pkgconfig stubs.
            ignoreCollisions = true;
            postBuild = ''
              # Replace the LLVM-only llvm-config with a wrapper that reports
              # this joined prefix so Melior/mlir-sys find libMLIR + mlir-c.
              real_llvm_config="${llvmPkgs.llvm.dev}/bin/llvm-config"
              if [ ! -x "$real_llvm_config" ]; then
                real_llvm_config="${llvmPkgs.llvm}/bin/llvm-config"
              fi
              rm -f "$out/bin/llvm-config"
              {
                echo '#!/bin/sh'
                echo "JOINED=\"$out\""
                echo "REAL=\"$real_llvm_config\""
                echo 'if [ "$#" -eq 1 ]; then'
                echo '  case "$1" in'
                echo '    --prefix) echo "$JOINED"; exit 0 ;;'
                echo '    --bindir) echo "$JOINED/bin"; exit 0 ;;'
                echo '    --libdir) echo "$JOINED/lib"; exit 0 ;;'
                echo '    --includedir) echo "$JOINED/include"; exit 0 ;;'
                echo '    --cmakedir) echo "$JOINED/lib/cmake/llvm"; exit 0 ;;'
                echo '  esac'
                echo 'fi'
                echo 'exec "$REAL" "$@"'
              } > "$out/bin/llvm-config"
              chmod +x "$out/bin/llvm-config"
            '';
            meta = with pkgs.lib; {
              description = "symlinkJoin of LLVM 22 + MLIR 22 for Melior";
              platforms = platforms.unix;
            };
          };
        in
        {
          default = llvm-mlir;
          llvm-mlir = llvm-mlir;
        }
      );

      # Convenience: `nix develop` exposes the same prefix on PATH.
      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          llvm-mlir = self.packages.${system}.default;
        in
        {
          default = pkgs.mkShell {
            packages = [
              llvm-mlir
              pkgs.z3
              pkgs.pkg-config
            ];
            shellHook = ''
              export MLIR_SYS_220_PREFIX="$(llvm-config --prefix)"
              export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
            '';
          };
        }
      );
    };
}
