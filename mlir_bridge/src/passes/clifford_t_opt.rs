//! Clifford+T optimization pass (issue #21).
//!
//! Applies peephole self-inverse cancellation (Pauli pairs, T·T†, CNOT·CNOT, etc.)
//! via [`super::gate_cancellation`] for both Clifford and universal circuits.

use melior::ir::OperationRef;
use melior::ir::r#type::TypeId;
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef};

use crate::passes::gate_cancellation;

/// Runs Clifford+T optimization on `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    gate_cancellation::run_on_module(context, module);
}

#[repr(align(8))]
struct PassId;

static CLIFFORD_T_OPT_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct CliffordTOpt {
    context: usize,
}

impl CliffordTOpt {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for CliffordTOpt {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        gate_cancellation::cancel_module(context, operation);
    }
}

/// Creates the Clifford+T optimization pass.
pub fn create_pass() -> Pass {
    create_external(
        CliffordTOpt::new(),
        TypeId::create(&CLIFFORD_T_OPT_PASS_ID),
        "clifford-t-opt",
        "clifford-t-opt",
        "Clifford+T peephole optimization via gate cancellation",
        "",
        &[],
    )
}
