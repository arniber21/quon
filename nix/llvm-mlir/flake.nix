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
              # It must rewrite paths for ANY invocation, not just single-flag
              # ones: mlir-sys queries e.g.
              #   llvm-config --link-static --ignore-libllvm --includedir
              # so we run the real llvm-config and substitute every LLVM/MLIR
              # store path in its output with the joined prefix (which
              # symlinks all of them).
              real_llvm_config="${llvmPkgs.llvm.dev}/bin/llvm-config"
              if [ ! -x "$real_llvm_config" ]; then
                real_llvm_config="${llvmPkgs.llvm}/bin/llvm-config"
              fi
              rm -f "$out/bin/llvm-config"
              {
                echo '#!/bin/sh'
                echo "JOINED=\"$out\""
                echo "REAL=\"$real_llvm_config\""
                echo "LLVM_DEV=\"${llvmPkgs.llvm.dev}\""
                echo "LLVM_LIB=\"${llvmPkgs.llvm.lib}\""
                echo "LLVM_OUT=\"${llvmPkgs.llvm}\""
                echo "MLIR_DEV=\"${llvmPkgs.mlir.dev}\""
                echo "MLIR_OUT=\"${llvmPkgs.mlir}\""
                echo 'res=$("$REAL" "$@") || exit $?'
                echo 'printf "%s\n" "$res" | sed -e "s|$LLVM_DEV|$JOINED|g" -e "s|$LLVM_LIB|$JOINED|g" -e "s|$LLVM_OUT|$JOINED|g" -e "s|$MLIR_DEV|$JOINED|g" -e "s|$MLIR_OUT|$JOINED|g"'
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
