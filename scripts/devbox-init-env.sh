#!/usr/bin/env bash
# Devbox init_hook body (sourced from devbox.json). Exports the toolchain env
# for building and *running* workspace binaries inside the devbox environment.
#
# Runtime library paths matter because binaries built with the nix cc wrapper
# use the nix dynamic linker, which searches only rpath + *_LIBRARY_PATH — it
# never falls back to /usr/lib, so nix-provided shared libraries (libz3, and
# the gcc runtime `z3-sys` links via `-lstdc++`) must be exported explicitly.
# Always export the *resolved store dir* of the exact library, never a merged
# profile lib dir: path-order lookup by leaf name would otherwise make nix
# tools load the flake's libLLVM/libstdc++ instead of their own (the macOS
# DYLD_LIBRARY_PATH incident, 2026-07-13).

export MLIR_SYS_220_PREFIX="$(llvm-config --prefix)"
export PATH="$MLIR_SYS_220_PREFIX/bin:$PATH"
export LIBCLANG_PATH="$MLIR_SYS_220_PREFIX/lib"

_z3_prefix="$(pkg-config --variable=prefix z3 2>/dev/null || true)"
if [ -z "$_z3_prefix" ] && command -v z3 >/dev/null 2>&1; then
  _z3_bin="$(command -v z3)"
  _z3_prefix="$(cd "$(dirname "$_z3_bin")/.." && pwd)"
fi

if [ -n "$_z3_prefix" ]; then
  export LIBRARY_PATH="$_z3_prefix/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
  export BINDGEN_EXTRA_CLANG_ARGS="-I$_z3_prefix/include ${BINDGEN_EXTRA_CLANG_ARGS:-}"
  if [ "$(uname -s)" = Darwin ]; then
    _z3_libdir="$_z3_prefix/lib"
    if [ -e "$_z3_libdir/libz3.dylib" ]; then
      _z3_libdir="$(dirname "$(readlink -f "$_z3_libdir/libz3.dylib")")"
    fi
    export DYLD_FALLBACK_LIBRARY_PATH="$_z3_libdir${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
  else
    _z3_libdir="$_z3_prefix/lib"
    if [ -e "$_z3_libdir/libz3.so" ]; then
      _z3_libdir="$(dirname "$(readlink -f "$_z3_libdir/libz3.so")")"
    fi
    export LD_LIBRARY_PATH="$_z3_libdir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

    # gcc runtime (libstdc++/libgcc_s) for binaries that link `-lstdc++`
    # directly (z3-sys). Prefer what the toolchain reports; fall back to the
    # gcc-lib dir recorded in libz3.so's own RUNPATH (readelf ships with the
    # GitHub runners and any binutils install).
    _cxx_libdir=""
    _cxx_lib="$(cc -print-file-name=libstdc++.so.6 2>/dev/null || true)"
    if [ -n "$_cxx_lib" ] && [ "$_cxx_lib" != "libstdc++.so.6" ] && [ -e "$_cxx_lib" ]; then
      _cxx_libdir="$(dirname "$(readlink -f "$_cxx_lib")")"
    elif command -v readelf >/dev/null 2>&1 && [ -e "$_z3_libdir/libz3.so" ]; then
      _rpaths="$(readelf -d "$_z3_libdir/libz3.so" 2>/dev/null \
        | sed -n 's/.*R\(UN\)\{0,1\}PATH.*\[\(.*\)\]/\2/p' | tr ':' '\n')"
      for _rp in $_rpaths; do
        if [ -e "$_rp/libstdc++.so.6" ]; then
          _cxx_libdir="$_rp"
          break
        fi
      done
      unset _rpaths _rp
    fi
    if [ -n "$_cxx_libdir" ] && [ "$_cxx_libdir" != "$_z3_libdir" ]; then
      export LD_LIBRARY_PATH="$_cxx_libdir:$LD_LIBRARY_PATH"
    fi
    unset _cxx_lib _cxx_libdir
  fi
fi

# zlib for manylinux wheels (numpy) imported by the nix python: the wheel's
# C extension NEEDs libz.so.1, which the nix loader won't find in /usr/lib.
# zlib's .pc file points at the *static* output, so resolve the shared lib
# through the merged devbox profile instead (export the resolved store dir,
# not the profile dir — see the leaf-name warning above).
if [ "$(uname -s)" != Darwin ]; then
  _profile_zlib="${DEVBOX_PROJECT_ROOT:-$PWD}/.devbox/nix/profile/default/lib/libz.so.1"
  if [ -e "$_profile_zlib" ]; then
    _zlib_libdir="$(dirname "$(readlink -f "$_profile_zlib")")"
    export LD_LIBRARY_PATH="$_zlib_libdir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  fi
  unset _profile_zlib _zlib_libdir
fi

unset _z3_prefix _z3_bin _z3_libdir
