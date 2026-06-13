// mlir_bridge build script.
// Melior handles LLVM/MLIR library linking automatically.
//
// Prerequisites:
//   LLVM 19 built with -DLLVM_ENABLE_PROJECTS=mlir and the C API enabled.
//
// If LLVM 19 is not on the default search path, set one of:
//   LLVM_SYS_PREFIX=/path/to/llvm19
//   MLIR_SYS_PREFIX=/path/to/llvm19

fn main() {
    // Melior's own build script emits the correct cargo:rustc-link-lib directives.
    // Nothing additional required here unless linking against extra LLVM components.
    println!("cargo:rerun-if-env-changed=LLVM_SYS_PREFIX");
    println!("cargo:rerun-if-env-changed=MLIR_SYS_PREFIX");
}
