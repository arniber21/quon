// mlir_bridge build script.
// Melior handles LLVM/MLIR library linking automatically.
//
// Prerequisites:
//   LLVM 22 built with -DLLVM_ENABLE_PROJECTS=mlir and the C API enabled.
//
// If LLVM 22 is not on the default search path, set MLIR_SYS_220_PREFIX.

fn main() {
    // Melior's own build script emits the correct cargo:rustc-link-lib directives.
    // Nothing additional required here unless linking against extra LLVM components.
    println!("cargo:rerun-if-env-changed=MLIR_SYS_220_PREFIX");
}
