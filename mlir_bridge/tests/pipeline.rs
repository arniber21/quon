//! Smoke tests: Fixed pipeline stages are callable without `quonc` (#210).

mod support;

use melior::ir::{Block, BlockLike, Location, Module, Region, RegionLike, Value};

use mlir_bridge::dialect::quantum_circ as qc;
use mlir_bridge::passes::sabre_routing::SabreCost;
use mlir_bridge::pipeline::{run_circ_passes_to_fixpoint, run_dynamic_passes, run_fixed_physical};
use quon_core::DepthExpr;

use support::context;

fn hh_module(context: &melior::Context) -> Module<'_> {
    let location = Location::unknown(context);
    let qubit = qc::qubit_type(context);
    let block = Block::new(&[(qubit, location)]);
    let mut wire = Value::from(block.argument(0).expect("arg"));
    for _ in 0..2 {
        let op = block
            .append_operation(qc::gate(context, "H", 1, true, &[wire], location).expect("gate"));
        wire = Value::from(op.result(0).expect("result"));
    }
    block.append_operation(qc::r#return(&[wire], location).expect("return"));
    let region = Region::new();
    region.append_block(block);
    let func = qc::func(
        context,
        "main",
        1,
        1,
        &DepthExpr::Nat(2),
        true,
        region,
        location,
    )
    .expect("func");
    let module = Module::new(location);
    module.body().append_operation(func);
    module
}

#[test]
fn circ_fixpoint_cancels_hh_without_quonc() {
    let context = context();
    let module = hh_module(&context);
    let events = capture_warns(|| {
        assert!(
            run_circ_passes_to_fixpoint(&context, &module),
            "H·H should converge before the fixpoint cap"
        );
    });
    let text = module.as_operation().to_string();
    assert!(
        !text.contains("gate_name = \"H\""),
        "expected H·H cancelled via pipeline fixpoint: {text}"
    );
    assert!(
        events.is_empty(),
        "a converging circuit must not warn about the fixpoint cap: {events:?}"
    );
}

#[test]
fn circ_fixpoint_cap_warns_with_pass_names_and_rounds() {
    let context = context();
    let module = hh_module(&context);
    let events = capture_warns(|| {
        assert!(
            !mlir_bridge::pipeline::run_circ_passes_to_fixpoint_bounded(&context, &module, 1),
            "one round of H·H cancellation still changes the module"
        );
    });
    let text = events.join("\n");
    assert!(
        text.contains("quantum.circ optimization fixpoint was not reached"),
        "cap must emit a warning: {text}"
    );
    assert!(
        text.contains("rounds=1"),
        "warning must include the round count: {text}"
    );
    for pass in [
        "gate_cancellation",
        "rotation_merging",
        "clifford_t_opt",
        "compiler_uncomputation",
        "zx_simplification",
    ] {
        assert!(text.contains(pass), "warning must name {pass}: {text}");
    }
}

fn capture_warns(body: impl FnOnce()) -> Vec<String> {
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let subscriber = WarnCapture {
        events: std::sync::Arc::clone(&events),
    };
    tracing::subscriber::with_default(subscriber, body);
    events.lock().expect("warn capture").clone()
}

struct WarnCapture {
    events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

struct FieldVisit {
    parts: Vec<String>,
}

impl tracing::field::Visit for FieldVisit {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.parts.push(format!("{}={value:?}", field.name()));
    }
}

impl tracing::Subscriber for WarnCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        metadata.target() == "mlir_bridge::pipeline"
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() > tracing::Level::WARN {
            return;
        }
        let mut visit = FieldVisit { parts: Vec::new() };
        event.record(&mut visit);
        self.events
            .lock()
            .expect("warn capture")
            .push(visit.parts.join(" "));
    }

    fn enter(&self, _span: &tracing::span::Id) {}

    fn exit(&self, _span: &tracing::span::Id) {}
}

#[test]
fn fixed_physical_runs_on_emptyish_module() {
    // Dynamic + physical on a circ-only module: lowering is out of scope
    // here; just ensure the physical orchestration entry point is callable.
    let context = context();
    let module = Module::new(Location::unknown(&context));
    run_dynamic_passes(&context, &module);
    let target = backend::generic_openqasm::target(4);
    let _ = run_fixed_physical(&context, &target, SabreCost::default(), &module);
}
