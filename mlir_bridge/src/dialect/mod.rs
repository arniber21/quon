pub mod quantum_circ;
pub mod quantum_dynamic;

/// Registers every MLIR dialect implemented in this crate.
pub fn register_all(context: &melior::Context) {
    quantum_circ::register_dialect(context);
    quantum_dynamic::register_dialect(context);
}
