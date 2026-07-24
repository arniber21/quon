use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anstyle::{AnsiColor, Color, Style};
use anyhow::{Context as _, Result, anyhow, bail};
use clap::{ArgAction, ColorChoice, Parser, ValueEnum};
use quon_core::{
    RegressionConfig, compare, format_comparison_table, format_metrics_line, load_snapshot,
    save_snapshot,
};
use quon_na::{
    NeutralAtomAction, PlacementStrategy, PlacerMode, attach_qec_error_budget, na_stats_to_json,
    require_target_error_model, resource_report_to_json, resource_report_to_markdown,
    round_barrier_cuts,
};
use sha2::{Digest, Sha256};

use backend::{BackendTarget, TargetKind};
use quon_na::pipeline::{NaObjective, StatePrepMode};
use quon_qec::{
    attach_barrier_cycles, dual_emit, expand_workload, experiment_to_json, na_refs_from_expanded,
    sibling_stim_path,
};
use quonc::compile::{
    CompileRequest, build_na_schedule_view, compile, schedule_to_json, schedule_to_mlir,
};
use quonc::na_target::{
    NaBackendKind, parse_na_backend, parse_na_objective, parse_placer_mode, parse_state_prep_mode,
};
use quonc::watch::{print_watch_metrics, run_watch_loop};

const AFTER_HELP: &str = "\
Examples:
  # Fixed / OpenQASM path
  quonc program.qn --emit-qasm
  quonc program.qn --target targets/ibm/fake_manila.json --emit-qasm --metrics

  # Neutral-atom path (#112, #167): quantum.na MLIR is the primary artifact
  quonc program.qn --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-na-mlir

  # Neutral-atom debug artifacts: schedule JSON + interaction graph DOT
  quonc program.qn --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-na-schedule --emit-na-graph --emit-resource-report

  # OpenQASM 2/3 ingestion for the NA pipeline (#304): a .qasm extension
  # (or --from-qasm) bypasses .qn lowering and enters at the interaction graph
  quonc circuit.qasm --target targets/neutral_atom/rap_table_i.json \\
    --emit-resource-report -

  # MQT NAViz interop (#303): .naviz instructions + sibling .namachine
  quonc program.qn --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-naviz /tmp/ghz.naviz

  # Compiler statistics: per-stage wall times, search diagnostics, config
  # echo (#307) — a separate artifact from --emit-resource-report
  quonc program.qn --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-na-stats -

  # QEC experiment: semantic *.qec.json + sibling structure-level .stim
  quonc examples/na_qec/repetition_d3_memory.qn \\
    --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-qec-experiment /tmp/rep_d3.qec.json

  # QEC validation: fuse analytic + Stim/Sinter sampled evidence (#280).
  # Compiles, emits the QEC experiment, shells out to python/quon_qec_sinter.py,
  # and writes /tmp/rep_d3.validation.json (+ .md) with separate sections.
  quonc examples/na_qec/repetition_d3_memory.qn \\
    --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-qec-validation /tmp/rep_d3.validation.json --validation-shots 256

  # Debug IR after each pass
  quonc program.qn --dump-ir --verify-linear --emit-qasm

  # Verify quantum.na (standalone MLIR or after emit; QEC auto-verifies)
  quonc schedule.mlir --verify-na
  quonc program.qn --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-na-mlir --verify-na

  # Inspect a target without compiling
  quonc --target targets/neutral_atom/generic_rna_v0.json --print-target

Notes:
  Fixed targets run SABRE routing and emit OpenQASM 3.0.
  Neutral-atom targets extract an interaction graph, schedule entangling
  layers, run zoned RAP (default) or flat AOD movement, optionally compact,
  then lower to quantum.na MLIR (--emit-na-mlir, the canonical schedule IR).
  Schedule JSON (--emit-na-schedule) is a debug/visualization view
  (layout + zones + metrics envelope for python/visualize_na_schedule.py).
  --emit-na-graph writes Graphviz DOT for the interaction graph.
  --emit-na-stats writes compiler-internals telemetry (timings / search
  diagnostics / effective config), never schedule or QEC evidence — kept
  separate from --emit-resource-report on purpose. Available for both
  bare-qubit and QEC-backed programs (#317).
  --emit-qec-experiment writes QEC evaluation JSON + structure-only Stim
  (no physical noise; Python annotates from JSON error_model).
";

#[derive(Parser, Debug)]
#[command(
    name = "quonc",
    about = "Quon quantum compiler",
    long_about = "Quon quantum compiler — OpenQASM 3.0 and neutral-atom schedules.\n\n\
Compile Quon programs through the MLIR pipeline. Fixed (gate-model) targets \
emit OpenQASM 3.0. Neutral-atom reconfigurable targets schedule AOD movement \
/ zoned RAP and emit quantum.na MLIR (the canonical schedule IR), plus \
schedule JSON and resource reports as debug artifacts.",
    after_help = AFTER_HELP,
    version,
    color = ColorChoice::Auto,
    styles = clap_styles(),
    propagate_version = true,
    arg_required_else_help = true,
    help_template = "\
{before-help}{name} {version}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}"
)]
struct Cli {
    /// Source file to compile (`.qn` Quon program, or `.qasm` OpenQASM 2/3
    /// for the neutral-atom ingestion path, #304). Optional with --print-target
    /// / --list-passes.
    source: Option<PathBuf>,

    /// Parse the source as OpenQASM 2/3 (#304) instead of Quon (forces the
    /// neutral-atom path; a `.qasm` extension already implies this).
    #[arg(long, help_heading = "Input", action = ArgAction::SetTrue)]
    from_qasm: bool,

    // ── Emit ────────────────────────────────────────────────────────────
    /// Emit OpenQASM 3.0 (fixed targets only)
    #[arg(long, help_heading = "Emit", action = ArgAction::SetTrue)]
    emit_qasm: bool,

    /// Emit quantum.na MLIR, the primary neutral-atom artifact (`-` = stdout)
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_na_mlir: Option<String>,

    /// Emit neutral-atom schedule JSON envelope for visualization (`-` = stdout)
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_na_schedule: Option<String>,

    /// Emit interaction-graph Graphviz DOT (`-` = stdout)
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_na_graph: Option<String>,

    /// Emit analytic NA resource report (`-` = stdout; `.md` → Markdown, else JSON; not Sinter CSV — ADR-0020)
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_resource_report: Option<String>,

    /// Emit neutral-atom compiler statistics (per-stage wall times, search
    /// diagnostics, effective config echo; `-` = stdout). A separate artifact
    /// from --emit-resource-report (issue #307) — compiler-internals
    /// telemetry about the compile, not schedule/QEC evidence.
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_na_stats: Option<String>,

    /// Emit MQT NAViz interop artifacts: a `.naviz` instruction file plus a
    /// sibling `.namachine` (zones, SLM traps, rydberg range) for rendering in
    /// the MQT NAViz visualizer (#303). Requires a filesystem PATH (writes two
    /// sibling files); stdout is not supported.
    #[arg(long, value_name = "PATH", help_heading = "Emit")]
    emit_naviz: Option<PathBuf>,

    /// Emit QEC experiment JSON + sibling structure-level `.stim` (ADR-0018)
    #[arg(long, value_name = "PATH", help_heading = "Emit")]
    emit_qec_experiment: Option<PathBuf>,

    /// Emit a fused QEC validation report (`*.validation.json` + `.md`): compiles,
    /// emits the QEC experiment, shells out to the Python Stim/Sinter harness, and
    /// fuses analytic + sampled evidence into separate sections (#280 / ADR-0020)
    #[arg(long, value_name = "PATH", help_heading = "Emit")]
    emit_qec_validation: Option<PathBuf>,

    /// Force resource-report format (overrides PATH extension)
    #[arg(long, value_enum, value_name = "FMT", help_heading = "Emit")]
    resource_report_format: Option<ReportFormat>,

    // ── Target ──────────────────────────────────────────────────────────
    /// Backend target descriptor (JSON). Defaults to generic_openqasm.
    #[arg(long, help_heading = "Target")]
    target: Option<PathBuf>,

    /// Print the loaded backend target summary and exit
    #[arg(long, help_heading = "Target", action = ArgAction::SetTrue)]
    print_target: bool,

    // ── Neutral atom ────────────────────────────────────────────────────
    /// NA movement backend: zoned (RAP, default) or flat (AOD pair-bank)
    #[arg(
        long,
        value_name = "KIND",
        default_value = "zoned",
        help_heading = "Neutral atom",
        value_parser = parse_na_backend
    )]
    na_backend: NaBackendKind,

    /// Zoned placer mode: routing-agnostic (default), routing-aware, or exact
    /// (SMT-optimal placement, requires the `solver` feature)
    #[arg(
        long,
        value_name = "MODE",
        default_value = "routing-agnostic",
        help_heading = "Neutral atom",
        value_parser = parse_placer_mode
    )]
    na_placer: PlacerMode,

    /// Skip schedule compaction after NA movement/zoned scheduling
    #[arg(long, help_heading = "Neutral atom", action = ArgAction::SetTrue)]
    no_na_compact: bool,

    /// Flat AOD placement strategy
    #[arg(
        long,
        value_enum,
        default_value_t = CliPlacement::RowMajor,
        help_heading = "Neutral atom"
    )]
    na_placement: CliPlacement,

    /// State-preparation scheduling mode: heuristic (default) or exact
    /// (SMT-optimal CZ-pair scheduling, requires the `solver` feature)
    #[arg(
        long,
        value_name = "MODE",
        default_value = "heuristic",
        help_heading = "Neutral atom",
        value_parser = parse_state_prep_mode
    )]
    na_state_prep: StatePrepMode,
    /// Placement objective: time (default, minimizes Σ √(d_max)) or
    /// error-budget (minimizes analytic error_model contributions —
    /// requires the target's error_model; ADR-0017/0020, not logical rates)
    #[arg(
        long,
        value_name = "OBJECTIVE",
        default_value = "time",
        help_heading = "Neutral atom",
        value_parser = parse_na_objective
    )]
    na_objective: NaObjective,
    // ── Debug ───────────────────────────────────────────────────────────
    /// Dump MLIR after each major pass stage to stderr
    #[arg(long, help_heading = "Debug", action = ArgAction::SetTrue)]
    dump_ir: bool,

    /// Run circ/dynamic linearity verifiers (debug)
    #[arg(long, help_heading = "Debug", action = ArgAction::SetTrue)]
    verify_linear: bool,

    /// Verify `quantum.na` schedule legality (occupancy, Rydberg, AOD, measure/reset
    /// ordering, Wait hard schedule barriers). Feed-forward / correction ordering
    /// is compaction-only (out of scope for #256). Auto-runs for any QEC-backed
    /// NA compile (ADR-0021); physical NA requires this flag. Also accepts
    /// standalone `.mlir`.
    #[arg(long, help_heading = "Neutral atom", action = ArgAction::SetTrue)]
    verify_na: bool,

    /// List compiler pass stages and exit
    #[arg(long, help_heading = "Debug", action = ArgAction::SetTrue)]
    list_passes: bool,

    /// Quiet: suppress the “compiled successfully” hint
    #[arg(short = 'q', long, help_heading = "Debug", action = ArgAction::SetTrue)]
    quiet: bool,

    /// Colorize diagnostics and help (auto|always|never)
    #[arg(
        long,
        value_enum,
        default_value_t = CliColor::Auto,
        help_heading = "Debug",
        env = "QUONC_COLOR"
    )]
    color: CliColor,

    // ── Metrics / watch ─────────────────────────────────────────────────
    /// Print a human-readable metrics summary to stderr
    #[arg(long, help_heading = "Metrics", action = ArgAction::SetTrue)]
    metrics: bool,

    /// Write metrics JSON to file (`-` for stdout/stderr — see `--emit-qasm`)
    #[arg(long, value_name = "PATH", help_heading = "Metrics")]
    metrics_json: Option<String>,

    /// Save or compare a metrics snapshot baseline (`save PATH` or `compare PATH`)
    #[arg(
        long,
        num_args = 2,
        value_names = ["ACTION", "PATH"],
        help_heading = "Metrics"
    )]
    metrics_snapshot: Option<Vec<String>>,

    /// TOML/JSON tolerance file for `--metrics-snapshot compare`
    #[arg(long, value_name = "PATH", help_heading = "Metrics")]
    regression_config: Option<PathBuf>,

    /// Watch source (and `--target` if set) for changes; recompile on change
    #[arg(long, help_heading = "Metrics", action = ArgAction::SetTrue)]
    watch: bool,

    /// Debounce window for filesystem events in watch mode (milliseconds)
    #[arg(long, default_value_t = 300, help_heading = "Metrics")]
    watch_debounce_ms: u64,

    /// SABRE noise-weight coefficient γ (SPEC §7.4). Fixed targets only.
    #[arg(long, default_value_t = 0.3, help_heading = "Target")]
    sabre_gamma: f64,

    /// SABRE critical-path coefficient β (SPEC §7.4). Fixed targets only.
    #[arg(long, default_value_t = 0.5, help_heading = "Target")]
    sabre_beta: f64,

    /// SABRE lookahead window size (SPEC §7.4). Fixed targets only.
    #[arg(long, default_value_t = 20, help_heading = "Target")]
    sabre_lookahead: usize,

    // ── QEC validation (#280) ───────────────────────────────────────────
    /// Sinter shots for `--emit-qec-validation`
    #[arg(
        long,
        default_value_t = 64,
        value_name = "N",
        help_heading = "QEC validation"
    )]
    validation_shots: u64,

    /// Stim detector-sampler seed for `--emit-qec-validation` (deterministic)
    #[arg(
        long,
        default_value_t = 7,
        value_name = "SEED",
        help_heading = "QEC validation"
    )]
    validation_seed: i64,

    /// Sinter decoder for `--emit-qec-validation`
    #[arg(
        long,
        default_value = "pymatching",
        value_name = "NAME",
        help_heading = "QEC validation"
    )]
    validation_decoder: String,

    /// Attach a pre-sampled evidence JSON (from `quon_qec_sinter.py --json`)
    /// instead of shelling out to Python (`--emit-qec-validation`)
    #[arg(long, value_name = "PATH", help_heading = "QEC validation")]
    attach_sampled: Option<PathBuf>,

    /// Warn (do not refuse) when sampled data provenance mismatches the artifact
    #[arg(long, action = ArgAction::SetTrue, help_heading = "QEC validation")]
    allow_sampled_mismatch: bool,

    /// Python interpreter for the Stim/Sinter harness (default: repo `.venv` then `python3`)
    #[arg(long, value_name = "PATH", help_heading = "QEC validation")]
    python: Option<PathBuf>,

    /// Path to `quon_qec_sinter.py` (default: search up from CWD for `python/`)
    #[arg(long, value_name = "PATH", help_heading = "QEC validation")]
    sinter_harness: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ReportFormat {
    Json,
    Markdown,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum CliColor {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum CliPlacement {
    #[default]
    #[value(name = "row-major")]
    RowMajor,
    #[value(name = "degree")]
    DegreeBased,
    #[value(name = "clustering")]
    InteractionClustering,
}

impl From<CliPlacement> for PlacementStrategy {
    fn from(value: CliPlacement) -> Self {
        match value {
            CliPlacement::RowMajor => PlacementStrategy::RowMajor,
            CliPlacement::DegreeBased => PlacementStrategy::DegreeBased,
            CliPlacement::InteractionClustering => PlacementStrategy::InteractionClustering,
        }
    }
}

const fn clap_styles() -> clap::builder::Styles {
    clap::builder::Styles::styled()
        .header(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::BrightBlue))),
        )
        .usage(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::BrightBlue))),
        )
        .literal(Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightCyan))))
        .placeholder(Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightGreen))))
        .error(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::BrightRed))),
        )
        .valid(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::BrightGreen))),
        )
        .invalid(Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightYellow))))
}

#[derive(Clone, Copy, Debug)]
enum SnapshotAction {
    Save,
    Compare,
}

fn parse_snapshot_action(action: &str) -> Result<SnapshotAction> {
    match action.to_ascii_lowercase().as_str() {
        "save" => Ok(SnapshotAction::Save),
        "compare" => Ok(SnapshotAction::Compare),
        other => Err(anyhow::anyhow!(
            "unknown metrics-snapshot action `{other}` (expected save or compare)"
        )),
    }
}

fn snapshot_from_parts(parts: &[String]) -> Result<(SnapshotAction, PathBuf)> {
    let [action, path] = parts else {
        bail!("--metrics-snapshot requires ACTION and PATH");
    };
    Ok((parse_snapshot_action(action)?, PathBuf::from(path)))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            let style = error_style();
            eprintln!("{style}error{style:#}: {err:#}");
            ExitCode::from(2)
        }
    }
}

fn error_style() -> Style {
    if stderr_color_enabled() {
        Style::new()
            .bold()
            .fg_color(Some(Color::Ansi(AnsiColor::BrightRed)))
    } else {
        Style::new()
    }
}

fn ok_style() -> Style {
    if stderr_color_enabled() {
        Style::new().fg_color(Some(Color::Ansi(AnsiColor::BrightGreen)))
    } else {
        Style::new()
    }
}

fn dim_style() -> Style {
    if stderr_color_enabled() {
        Style::new().dimmed()
    } else {
        Style::new()
    }
}

fn stderr_color_enabled() -> bool {
    match std::env::var("QUONC_COLOR")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "always" | "1" | "true" => true,
        "never" | "0" | "false" => false,
        _ => io::stderr().is_terminal(),
    }
}

fn run() -> Result<ExitCode> {
    let mut cli = Cli::parse();
    apply_color_env(&cli);

    if cli.list_passes {
        print_pass_list();
        return Ok(ExitCode::SUCCESS);
    }

    let target = load_target(cli.target.as_ref())?;

    if cli.print_target {
        print!("{}", render_target_summary(&target));
        return Ok(ExitCode::SUCCESS);
    }

    // Standalone quantum.na MLIR verification (ADR-0021 / #256).
    if cli.verify_na
        && let Some(path) = &cli.source
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("mlir"))
    {
        return verify_na_mlir_file(path, cli.quiet);
    }

    validate_emit_flags(&cli, &target)?;

    if cli.watch && !cli.metrics {
        cli.metrics = true;
    }

    if cli.watch {
        return run_watch(&cli);
    }

    let request = build_request(&cli, target)?;
    let report = compile(&request);

    if report.snapshot.compile.status != quon_core::CompileStatus::Ok {
        if let Some(err) = &report.snapshot.compile.error {
            let style = error_style();
            eprintln!("{style}error{style:#}: {err}");
        }
        return Ok(ExitCode::from(1));
    }

    emit_artifacts(&cli, &request, &report)?;

    if report.na_schedule_spec.is_some() && (cli.verify_na || report.qec_backed) && !cli.quiet {
        let dim = dim_style();
        let kind = if report.qec_backed {
            "QEC auto"
        } else {
            "physical"
        };
        eprintln!("{dim}quantum.na verification passed ({kind}){dim:#}");
    }

    if cli.metrics {
        eprintln!("{}", format_metrics_line(&report.snapshot));
    }

    if let Some(path) = &cli.metrics_json {
        write_metrics_json(path, &report, cli.emit_qasm)?;
    }

    if let Some(parts) = &cli.metrics_snapshot {
        let (action, path) = snapshot_from_parts(parts)?;
        match action {
            SnapshotAction::Save => {
                save_snapshot(&path, &report.snapshot).map_err(|e| anyhow::anyhow!("{e}"))?;
                let ok = ok_style();
                eprintln!("{ok}saved snapshot{ok:#} → {}", path.display());
            }
            SnapshotAction::Compare => {
                let baseline = load_snapshot(&path).map_err(|e| anyhow::anyhow!("{e}"))?;
                let config = load_regression_config(cli.regression_config.as_ref())?;
                let comparison = compare(&baseline, &report.snapshot, &config)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
                eprintln!(
                    "{}",
                    format_comparison_table(&baseline, &report.snapshot, &comparison, &config)
                );
                if !comparison.passed {
                    return Ok(ExitCode::from(1));
                }
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

fn apply_color_env(cli: &Cli) {
    // clap ColorChoice is set at parse time via attribute; also honor --color for
    // our own stderr styling via QUONC_COLOR.
    match cli.color {
        CliColor::Always => unsafe { std::env::set_var("QUONC_COLOR", "always") },
        CliColor::Never => unsafe { std::env::set_var("QUONC_COLOR", "never") },
        CliColor::Auto => {}
    }
}

fn validate_emit_flags(cli: &Cli, target: &BackendTarget) -> Result<()> {
    let is_na = matches!(target.kind, TargetKind::NeutralAtomReconfigurable(_));
    if cli.emit_qasm && is_na {
        bail!(
            "--emit-qasm requires a fixed (gate-model) target; \
             use --emit-na-mlir (or --emit-na-schedule / --emit-na-graph / \
             --emit-resource-report / --emit-qec-experiment) for neutral-atom targets"
        );
    }
    if (cli.emit_na_mlir.is_some()
        || cli.emit_na_schedule.is_some()
        || cli.emit_na_graph.is_some()
        || cli.emit_resource_report.is_some()
        || cli.emit_na_stats.is_some()
        || cli.emit_naviz.is_some()
        || cli.emit_qec_experiment.is_some()
        || cli.emit_qec_validation.is_some())
        && !is_na
    {
        bail!(
            "--emit-na-mlir / --emit-na-schedule / --emit-na-graph / --emit-resource-report / \
             --emit-na-stats / --emit-naviz / --emit-qec-experiment / --emit-qec-validation \
             require a neutral_atom_reconfigurable target (see targets/neutral_atom/)"
        );
    }
    if cli.verify_na && !is_na {
        bail!(
            "--verify-na requires a neutral_atom_reconfigurable target \
             (or a standalone .mlir schedule with --verify-na)"
        );
    }
    if let Some(path) = &cli.emit_qec_experiment
        && path.as_os_str() == "-"
    {
        bail!(
            "--emit-qec-experiment requires a filesystem PATH (writes JSON + sibling .stim); \
             stdout dual-emit is not supported"
        );
    }
    if let Some(path) = &cli.emit_qec_validation
        && path.as_os_str() == "-"
    {
        bail!(
            "--emit-qec-validation requires a filesystem PATH (writes JSON report + sibling \
             .md and separate QEC / sampled primaries); stdout is not supported"
        );
    }
    if let Some(path) = &cli.emit_naviz
        && path.as_os_str() == "-"
    {
        bail!(
            "--emit-naviz requires a filesystem PATH (writes .naviz + sibling .namachine); \
             stdout dual-emit is not supported"
        );
    }
    Ok(())
}

fn emit_artifacts(
    cli: &Cli,
    request: &CompileRequest,
    report: &quonc::CompileReport,
) -> Result<()> {
    let mut emitted = false;
    // When OpenQASM already owns stdout, subsequent `-` emits go to stderr.
    let qasm_owns_stdout = cli.emit_qasm;

    if cli.emit_qasm {
        if let Some(qasm) = &report.qasm {
            print!("{qasm}");
            emitted = true;
        } else {
            bail!("OpenQASM emission produced no output (is the target fixed?)");
        }
    }

    // quantum.na MLIR is the primary NA artifact (ADR-0011): it takes stdout
    // ahead of the JSON debug view when both target `-`.
    if let Some(path) = &cli.emit_na_mlir {
        let spec = report.na_schedule_spec.as_ref().ok_or_else(|| {
            anyhow!("no quantum.na schedule available (compile with a neutral-atom target)")
        })?;
        let mlir = schedule_to_mlir(spec)?;
        // Prefer verifying the emitted text so dump drift cannot slip past the
        // in-memory `verify_schedule_spec` path (ADR-0021 nit).
        if cli.verify_na || report.qec_backed {
            quon_na::verify_mlir_text(&mlir)
                .map_err(|e| anyhow!("emitted quantum.na failed verification: {e}"))?;
        }
        write_output(path, &mlir, qasm_owns_stdout && path == "-")?;
        emitted = true;
    }
    let mlir_owns_stdout = cli.emit_na_mlir.as_ref().is_some_and(|p| p == "-");

    if let Some(path) = &cli.emit_na_schedule {
        let view = build_na_schedule_view(report, request)?;
        let json = schedule_to_json(&view)?;
        write_output(
            path,
            &json,
            (qasm_owns_stdout || mlir_owns_stdout) && path == "-",
        )?;
        emitted = true;
    }
    let schedule_on_stdout = cli.emit_na_schedule.as_ref().is_some_and(|p| p == "-");

    if let Some(path) = &cli.emit_na_graph {
        let graph = report.na_graph.as_ref().ok_or_else(|| {
            anyhow!("no interaction graph available (compile with a neutral-atom target)")
        })?;
        let dot = graph.to_dot();
        write_output(
            path,
            &dot,
            (qasm_owns_stdout || mlir_owns_stdout || schedule_on_stdout) && path == "-",
        )?;
        emitted = true;
    }
    let graph_on_stdout = cli.emit_na_graph.as_ref().is_some_and(|p| p == "-");

    if let Some(path) = &cli.emit_resource_report {
        let report_body = report.resource_report.as_ref().ok_or_else(|| {
            anyhow!("no resource report available (compile with a neutral-atom target)")
        })?;
        // ADR-0017: NA resource-report emit always attaches analytic error_budget
        // and hard-fails when the target has no error_model (never 1−fidelity).
        let na = match &request.target.kind {
            TargetKind::NeutralAtomReconfigurable(na) => na,
            _ => bail!(
                "--emit-resource-report requires a neutral_atom_reconfigurable target \
                 (see targets/neutral_atom/)"
            ),
        };
        let model = require_target_error_model(na).map_err(|e| anyhow!("{e}"))?;
        let report_body = attach_qec_error_budget(report_body.clone(), Some(model))
            .map_err(|e| anyhow!("{e}"))?;
        let text = match resolve_report_format(cli, path) {
            ReportFormat::Json => resource_report_to_json(&report_body)?,
            ReportFormat::Markdown => resource_report_to_markdown(&report_body),
        };
        // If MLIR / schedule / graph already printed to stdout on `-`, send the
        // report to stderr so all artifacts remain recoverable without
        // interleaving values.
        write_output(
            path,
            &text,
            (qasm_owns_stdout || mlir_owns_stdout || schedule_on_stdout || graph_on_stdout)
                && path == "-",
        )?;
        emitted = true;
    }
    let resource_report_on_stdout = cli.emit_resource_report.as_ref().is_some_and(|p| p == "-");

    if let Some(path) = &cli.emit_na_stats {
        let stats = report.na_stats.as_ref().ok_or_else(|| {
            anyhow!(
                "no NA compiler stats available for this compile (the neutral-atom \
                 pipeline failed to populate NaStats — this should not happen; see \
                 issue #307)"
            )
        })?;
        let json = na_stats_to_json(stats)?;
        // If an earlier artifact already printed to stdout on `-`, send stats
        // to stderr so all artifacts remain recoverable without interleaving.
        write_output(
            path,
            &json,
            (qasm_owns_stdout
                || mlir_owns_stdout
                || schedule_on_stdout
                || graph_on_stdout
                || resource_report_on_stdout)
                && path == "-",
        )?;
        emitted = true;
    }

    if let Some(path) = &cli.emit_naviz {
        let layers = report.na_schedule.as_ref().ok_or_else(|| {
            anyhow!("no neutral-atom schedule available (compile with a neutral-atom target)")
        })?;
        let layout = report.na_layout.as_ref().ok_or_else(|| {
            anyhow!("no neutral-atom layout available (compile with a neutral-atom target)")
        })?;
        let na = match &request.target.kind {
            TargetKind::NeutralAtomReconfigurable(na) => na,
            _ => bail!(
                "--emit-naviz requires a neutral_atom_reconfigurable target \
                 (see targets/neutral_atom/)"
            ),
        };
        // NAViz machine id = the sibling .namachine file-name stem.
        let machine_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("--emit-naviz requires a file path with a stem"))?;
        let namachine_path = path.with_extension("namachine");
        let naviz = quon_na::naviz::schedule_to_naviz(layers, layout, machine_id);
        let namachine = quon_na::naviz::target_to_namachine(na, layout, &request.target.id);
        write_atomic(path, &naviz)?;
        write_atomic(&namachine_path, &namachine)?;
        emitted = true;
    }

    if let Some(json_path) = &cli.emit_qec_experiment {
        emit_qec_experiment_artifacts(request, report, json_path)?;
        emitted = true;
    }

    if let Some(validation_path) = &cli.emit_qec_validation {
        emit_qec_validation(cli, request, report, validation_path)?;
        emitted = true;
    }

    if !emitted
        && cli.metrics_json.is_none()
        && cli.metrics_snapshot.is_none()
        && !cli.metrics
        && !cli.quiet
    {
        let dim = dim_style();
        match &report.snapshot.target.id {
            id if report.na_schedule.is_some() => {
                eprintln!(
                    "{dim}(compiled successfully for neutral-atom target `{id}`; \
                     pass --emit-na-mlir for quantum.na MLIR, or \
                     --emit-na-schedule / --emit-na-graph / --emit-resource-report / \
                     --emit-na-stats / --emit-naviz / --emit-qec-experiment / \
                     --emit-qec-validation for debug / QEC evaluation artifacts){dim:#}"
                );
            }
            id => {
                eprintln!(
                    "{dim}(compiled successfully for `{id}`; pass --emit-qasm to print OpenQASM 3.0){dim:#}"
                );
            }
        }
    }

    Ok(())
}

/// Dual-emit `*.qec.json` + sibling `.stim` from the same expanded QEC IR (ADR-0018).
fn emit_qec_experiment_artifacts(
    request: &CompileRequest,
    report: &quonc::CompileReport,
    json_path: &Path,
) -> Result<()> {
    build_and_write_qec_experiment(request, report, json_path)?;
    Ok(())
}

/// Dual-emit the QEC experiment pair and return the semantic experiment DTO.
///
/// Shared by `--emit-qec-experiment` and `--emit-qec-validation` (#280) so the
/// dual-emit contract (ADR-0018) has a single source of truth.
fn build_and_write_qec_experiment(
    request: &CompileRequest,
    report: &quonc::CompileReport,
    json_path: &Path,
) -> Result<quon_qec::QecExperiment> {
    let workload = report.qec_workload.as_ref().ok_or_else(|| {
        anyhow!(
            "--emit-qec-experiment requires a QEC-backed program (e.g. repetition_code / \
             memory_round); bare-qubit NA programs have no experiment IR"
        )
    })?;
    let na = match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => na,
        _ => bail!(
            "--emit-qec-experiment requires a neutral_atom_reconfigurable target \
             (see targets/neutral_atom/)"
        ),
    };
    // ADR-0017: hard-fail when error_model is missing (never invent rates).
    // Snapshot type is unified with quon_qec::ErrorModelSnapshot (backend alias).
    let model = require_target_error_model(na).map_err(|e| anyhow!("{e}"))?;
    let error_model = model.error_model_snapshot();

    // Re-expand from the same in-memory workload IR (never re-parse quantum.na).
    let expanded =
        expand_workload(workload).map_err(|e| anyhow!("QEC expand for experiment: {e}"))?;
    let stim_path = sibling_stim_path(json_path);
    let stim_basename = stim_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("experiment.stim")
        .to_string();

    let mut na_refs = na_refs_from_expanded(&expanded);
    if let Some(layers) = &report.na_schedule {
        let barriers = memory_round_barrier_cycles(layers, expanded.barrier_round_count())?;
        attach_barrier_cycles(&mut na_refs, &barriers).map_err(|e| anyhow!("{e}"))?;
    }

    let (experiment, stim) =
        dual_emit(&expanded, error_model, &stim_basename, na_refs).map_err(|e| anyhow!("{e}"))?;
    let json = experiment_to_json(&experiment).map_err(|e| anyhow!("{e}"))?;

    if let Some(parent) = json_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = stim_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut json_body = json;
    if !json_body.ends_with('\n') {
        json_body.push('\n');
    }
    let mut stim_body = stim;
    if !stim_body.ends_with('\n') {
        stim_body.push('\n');
    }

    // Atomic dual write: Stim first, then JSON; clean up Stim if JSON fails.
    write_atomic(&stim_path, &stim_body)
        .with_context(|| format!("write QEC Stim circuit {}", stim_path.display()))?;
    if let Err(e) = write_atomic(json_path, &json_body)
        .with_context(|| format!("write QEC experiment JSON {}", json_path.display()))
    {
        let _ = std::fs::remove_file(&stim_path);
        return Err(e);
    }

    Ok(experiment)
}

/// Compiler-driven QEC validation report (#280 / ADR-0020 amendment).
///
/// One user-facing entry point runs: compile (already done) → QEC experiment
/// dual-emit → analytic resource report → Stim/Sinter sampling (Python) →
/// provenance-checked fusion into a **separate** `*.validation.json` + `.md`.
/// Primary artifacts (QEC pair, resource report, sampled JSON) are kept beside
/// the report so evidence kinds stay separate (ADR-0020).
fn emit_qec_validation(
    cli: &Cli,
    request: &CompileRequest,
    report: &quonc::CompileReport,
    validation_path: &Path,
) -> Result<()> {
    let base = validation_base(validation_path);
    let qec_json_path = with_suffix(&base, ".qec.json");
    let resource_report_path = with_suffix(&base, ".resource_report.json");
    let sampled_path = with_suffix(&base, ".sampled.json");
    let markdown_path = with_suffix(&base, ".validation.md");

    // 1. QEC experiment dual-emit (analytic structure + sibling .stim).
    let experiment = build_and_write_qec_experiment(request, report, &qec_json_path)?;
    let qec_bytes = std::fs::read(&qec_json_path)
        .with_context(|| format!("read QEC experiment {}", qec_json_path.display()))?;
    let experiment_sha256 = sha256_hex_bytes(&qec_bytes);

    // 2. Analytic resource report (attach physical error budget, ADR-0017).
    let na = match &request.target.kind {
        TargetKind::NeutralAtomReconfigurable(na) => na,
        _ => bail!(
            "--emit-qec-validation requires a neutral_atom_reconfigurable target \
             (see targets/neutral_atom/)"
        ),
    };
    let resource_report = report.resource_report.as_ref().ok_or_else(|| {
        anyhow!("no resource report available (compile with a neutral-atom target)")
    })?;
    let model = require_target_error_model(na).map_err(|e| anyhow!("{e}"))?;
    let resource_report = attach_qec_error_budget(resource_report.clone(), Some(model))
        .map_err(|e| anyhow!("{e}"))?;
    let rr_json = resource_report_to_json(&resource_report)?;
    write_text_file(&resource_report_path, &rr_json)?;

    // 3. Sampled evidence: attach a pre-sampled JSON or shell out to Python.
    let sampled_text = if let Some(attach) = &cli.attach_sampled {
        std::fs::read_to_string(attach)
            .with_context(|| format!("read attached sampled JSON {}", attach.display()))?
    } else {
        run_sinter_harness(cli, &qec_json_path, &sampled_path)?;
        std::fs::read_to_string(&sampled_path)
            .with_context(|| format!("read sampled JSON {}", sampled_path.display()))?
    };
    let sampled: quonc::SampledEvidence = serde_json::from_str(&sampled_text)
        .with_context(|| "parse sampled evidence JSON (quon_qec_sinter.py --json output)")?;

    // 4. Fuse with provenance checking (refuse or warn on mismatch).
    let provenance = quonc::Provenance::from_experiment(
        &experiment,
        request.source_path.display().to_string(),
        request.target.id.clone(),
        experiment_sha256,
    );
    let fused = quonc::fuse(
        provenance,
        resource_report,
        sampled,
        cli.allow_sampled_mismatch,
    )
    .map_err(|e| anyhow!("{e}"))?;

    // 5. Write the separate JSON + Markdown validation artifacts.
    let json = quonc::validation_report_to_json(&fused)?;
    write_text_file(validation_path, &json)?;
    let md = quonc::validation_report_to_markdown(&fused);
    write_text_file(&markdown_path, &md)?;

    if !fused.mismatch_warnings.is_empty() {
        let style = error_style();
        eprintln!("{style}warning{style:#}: sampled data provenance mismatch (attached anyway):");
        for w in &fused.mismatch_warnings {
            eprintln!("  - {w}");
        }
    }

    if !cli.quiet {
        let ok = ok_style();
        eprintln!(
            "{ok}wrote QEC validation report{ok:#} → {} (+ .md; separate QEC / resource / sampled primaries)",
            validation_path.display()
        );
    }

    Ok(())
}

/// Strip a trailing `.validation.json` / `.json` / `.validation` to a stem path.
fn validation_base(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("report");
    let stem = name
        .strip_suffix(".validation.json")
        .or_else(|| name.strip_suffix(".json"))
        .or_else(|| name.strip_suffix(".validation"))
        .unwrap_or(name);
    path.with_file_name(stem)
}

/// Append `suffix` to the file name of `base` (e.g. `out` + `.qec.json`).
fn with_suffix(base: &Path, suffix: &str) -> PathBuf {
    let name = base
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("report");
    base.with_file_name(format!("{name}{suffix}"))
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn write_text_file(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut contents = body.to_string();
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    std::fs::write(path, contents).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Shell out to `python/quon_qec_sinter.py --json` to sample logical failures.
fn run_sinter_harness(cli: &Cli, qec_json: &Path, out_json: &Path) -> Result<()> {
    let repo_root = find_repo_root();
    let python = resolve_python(cli, repo_root.as_deref());
    let script = resolve_sinter_harness(cli, repo_root.as_deref())?;
    if let Some(parent) = out_json.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let output = Command::new(&python)
        .arg(&script)
        .arg(qec_json)
        .arg("--shots")
        .arg(cli.validation_shots.to_string())
        .arg("--seed")
        .arg(cli.validation_seed.to_string())
        .arg("--decoder")
        .arg(&cli.validation_decoder)
        .arg("--json")
        .arg(out_json)
        .output()
        .with_context(|| format!("run Stim/Sinter harness via {}", python.display()))?;
    if !output.status.success() {
        bail!(
            "Stim/Sinter harness failed ({}):\n{}\n(interpreter: {}, script: {})\n\
             Install evaluation deps (pip install -r python/requirements.txt / just setup-python), \
             or pass --python / --sinter-harness / --attach-sampled.",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim(),
            python.display(),
            script.display(),
        );
    }
    Ok(())
}

/// Walk up from the current directory for a repo containing the harness script.
fn find_repo_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("python/quon_qec_sinter.py").is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Resolve the Python interpreter: `--python` > `QUON_PYTHON` > repo `.venv` > `python3`.
fn resolve_python(cli: &Cli, repo_root: Option<&Path>) -> PathBuf {
    if let Some(p) = &cli.python {
        return p.clone();
    }
    if let Ok(env) = std::env::var("QUON_PYTHON")
        && !env.is_empty()
    {
        return PathBuf::from(env);
    }
    if let Some(root) = repo_root {
        let venv = root.join(".venv/bin/python");
        if venv.is_file() {
            return venv;
        }
    }
    PathBuf::from("python3")
}

/// Resolve the harness script: `--sinter-harness` > repo `python/quon_qec_sinter.py`.
fn resolve_sinter_harness(cli: &Cli, repo_root: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = &cli.sinter_harness {
        if p.is_file() {
            return Ok(p.clone());
        }
        bail!("--sinter-harness {} not found", p.display());
    }
    if let Some(root) = repo_root {
        let script = root.join("python/quon_qec_sinter.py");
        if script.is_file() {
            return Ok(script);
        }
    }
    bail!(
        "could not locate python/quon_qec_sinter.py (searched up from CWD); \
         pass --sinter-harness PATH, run from the repo root, or use --attach-sampled"
    );
}

/// Durable Wait barrier cycles from [`round_barrier_cuts`], fail-closed on count.
fn memory_round_barrier_cycles(
    layers: &[quon_na::ScheduleLayer],
    expected_memory_rounds: usize,
) -> Result<Vec<u32>> {
    let cuts = round_barrier_cuts(layers);
    let mut cycles = Vec::new();
    for &(idx, _) in &cuts {
        let layer = layers.get(idx as usize).ok_or_else(|| {
            anyhow!(
                "round_barrier_cuts index {idx} out of range ({} layers)",
                layers.len()
            )
        })?;
        let is_wait = layer
            .actions
            .iter()
            .any(|a| matches!(a, NeutralAtomAction::Wait { .. }));
        if is_wait {
            cycles.push(layer.cycle);
        }
    }
    if cycles.len() != expected_memory_rounds {
        bail!(
            "QEC na_refs barrier_cycle: found {} durable Wait barrier(s) via \
             round_barrier_cuts, expected {} barrier round(s); refusing unchecked Wait mapping",
            cycles.len(),
            expected_memory_rounds
        );
    }
    Ok(cycles)
}

fn write_atomic(path: &std::path::Path, contents: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("artifact");
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, contents).with_context(|| format!("write temp {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn resolve_report_format(cli: &Cli, path: &str) -> ReportFormat {
    if let Some(fmt) = cli.resource_report_format {
        return fmt;
    }
    if path != "-" && path.to_ascii_lowercase().ends_with(".md") {
        ReportFormat::Markdown
    } else {
        ReportFormat::Json
    }
}

fn write_output(path: &str, body: &str, prefer_stderr: bool) -> Result<()> {
    if path == "-" {
        if prefer_stderr {
            eprintln!("{body}");
        } else {
            io::stdout().write_all(body.as_bytes())?;
            if !body.ends_with('\n') {
                io::stdout().write_all(b"\n")?;
            }
        }
    } else {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut contents = body.to_string();
        if !contents.ends_with('\n') {
            contents.push('\n');
        }
        std::fs::write(&path, contents)?;
    }
    Ok(())
}

fn print_pass_list() {
    println!(
        "\
quonc pass stages
─────────────────
Shared front-end
  1. lower            Quon → quantum.dynamic (circ funcs + dynamic IR;
                      staging dialect collapsed in #213 / ADR-0037)
  2. circ fixpoint    gate_cancellation, rotation_merging,
                      clifford_t_opt (phase polynomial + stabilizer tableau),
                      compiler_uncomputation, zx_simplification
  3. dynamic          measurement_deferral, classical_region_fusion

Fixed (OpenQASM) path
  4. native_gate_decomp
  5. sabre_routing
  6. native_gate_decomp (post-SWAP)
  7. depth_scheduling
  8. emit OpenQASM 3.0

Neutral-atom path
  4. extract_interaction_graph
  5. schedule_entangling_layers (Misra–Gries / ASAP)
  6. schedule_zoned (default)  OR  place + plan_aod_movement (--na-backend flat)
  7. compact_schedule (unless --no-na-compact)
  8. lower to quantum.na MLIR (canonical schedule IR, ADR-0011)
  9. build_resource_report / schedule JSON (debug views)
"
    );
}

fn run_watch(cli: &Cli) -> Result<ExitCode> {
    let source = require_source(cli)?;
    let target = cli.target.clone();
    let debounce = cli.watch_debounce_ms;
    let metrics_json = cli.metrics_json.clone();
    let emit_qasm = cli.emit_qasm;
    let snapshot_compare = cli
        .metrics_snapshot
        .as_ref()
        .and_then(|parts| snapshot_from_parts(parts).ok())
        .and_then(|(action, path)| {
            if matches!(action, SnapshotAction::Compare) {
                Some(path)
            } else {
                None
            }
        });
    let regression_config_path = cli.regression_config.clone();
    let print_metrics = cli.metrics;

    let regression = snapshot_compare.map(|baseline_path| {
        let config = load_regression_config(regression_config_path.as_ref()).unwrap_or_default();
        (baseline_path, config)
    });

    let mut sticky_fail = false;

    run_watch_loop(
        source,
        target,
        debounce,
        || {
            let loaded = load_target(cli.target.as_ref())?;
            build_request(cli, loaded)
        },
        regression,
        |report, previous, comparison| {
            if print_metrics {
                print_watch_metrics(report, previous, comparison);
            }
            if let Some(path) = &metrics_json {
                let _ = write_metrics_json(path, report, emit_qasm);
            }
            if let Some(cmp) = comparison
                && !cmp.passed
            {
                sticky_fail = true;
            }
        },
    )?;

    if sticky_fail {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn load_target(path: Option<&PathBuf>) -> Result<BackendTarget> {
    match path {
        Some(path) => {
            backend::json::load(path).map_err(|e| anyhow!("loading target {}: {e}", path.display()))
        }
        None => Ok(backend::generic_openqasm::target(64)),
    }
}

fn require_source(cli: &Cli) -> Result<PathBuf> {
    cli.source.clone().ok_or_else(|| {
        anyhow!("missing source file; pass a .qn source, or use --print-target / --list-passes")
    })
}

fn build_request(cli: &Cli, target: BackendTarget) -> Result<CompileRequest> {
    let source_path = require_source(cli)?;
    let source = std::fs::read_to_string(&source_path)
        .with_context(|| format!("reading {}", source_path.display()))?;
    let from_qasm = cli.from_qasm
        || source_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("qasm"));

    Ok(CompileRequest {
        source_path,
        source,
        target,
        target_descriptor_path: cli.target.clone(),
        dump_ir: cli.dump_ir,
        verify_linear: cli.verify_linear,
        verify_na: cli.verify_na,
        sabre_gamma: cli.sabre_gamma,
        sabre_beta: cli.sabre_beta,
        sabre_lookahead: cli.sabre_lookahead,
        na_backend: cli.na_backend,
        na_placer: cli.na_placer,
        na_compact: !cli.no_na_compact,
        na_placement: cli.na_placement.into(),
        na_state_prep: cli.na_state_prep,
        na_objective: cli.na_objective,
        from_qasm,
    })
}

fn verify_na_mlir_file(path: &PathBuf, quiet: bool) -> Result<ExitCode> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    match quon_na::verify_mlir_text(&text) {
        Ok(()) => {
            if !quiet {
                let ok = ok_style();
                eprintln!("{ok}quantum.na verification passed{ok:#}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            let style = error_style();
            eprintln!("{style}error{style:#}: quantum.na verification failed: {err}");
            Ok(ExitCode::from(1))
        }
    }
}

fn load_regression_config(path: Option<&PathBuf>) -> Result<RegressionConfig> {
    match path {
        Some(p) => RegressionConfig::load(p).map_err(|e| anyhow::anyhow!("{e}")),
        None => Ok(RegressionConfig::default()),
    }
}

fn write_metrics_json(path: &str, report: &quonc::CompileReport, emit_qasm: bool) -> Result<()> {
    let json = serde_json::to_string_pretty(&report.snapshot)?;
    write_output(path, &json, emit_qasm)?;
    Ok(())
}

fn render_target_summary(target: &BackendTarget) -> String {
    let mut out = String::new();
    out.push_str(&format!("target: {}\n", target.id));
    out.push_str(&format!("kind: {}\n", target.kind_name()));

    match &target.kind {
        TargetKind::Fixed(fixed) => {
            out.push_str(&format!("num_qubits: {}\n", fixed.num_qubits));
            out.push_str(&format!("topology_edges: {}\n", fixed.topology.edges.len()));
            out.push_str(&format!(
                "native_gates: {}\n",
                fixed
                    .native_gates
                    .iter()
                    .map(|gate| gate.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            out.push_str(&format!(
                "measurement_latency_us: {}\n",
                fixed.meas_latency_us
            ));
            out.push_str(&format!(
                "supports_mid_circuit_meas: {}\n",
                fixed.supports_mid_circuit_meas
            ));
            out.push_str(&format!(
                "supports_feed_forward: {}\n",
                fixed.supports_feed_forward
            ));
        }
        TargetKind::NeutralAtomReconfigurable(na) => {
            out.push_str(&format!(
                "grid_um: {} x {}\n",
                na.grid.width_um, na.grid.height_um
            ));
            out.push_str(&format!("zones: {}\n", na.zones.len()));
            out.push_str(&format!(
                "zone_capacity: storage={}, entanglement={}, readout={}\n",
                na.zone_capacity(backend::ZoneKind::Storage),
                na.zone_capacity(backend::ZoneKind::Entanglement),
                na.zone_capacity(backend::ZoneKind::Readout)
            ));
            out.push_str(&format!(
                "movement: aod_row_column_coupled, rows={}, cols={}, aods={}, trap_transfer_us={}\n",
                na.movement.aod_rows,
                na.movement.aod_cols,
                na.movement.num_aods,
                na.movement.trap_transfer_us
            ));
            out.push_str(&format!(
                "rydberg: range_um={}, min_spacing_um={}, max_parallel_pairs={}\n",
                na.interaction.rydberg_range_um,
                na.interaction.min_rydberg_spacing_um,
                na.interaction.max_parallel_entangling_pairs
            ));
            out.push_str(&format!(
                "timing_us: cz={}, single_qubit={}, measurement={}, reset={}\n",
                na.timing.cz_us,
                na.timing.single_qubit_us,
                na.timing.measurement_us,
                na.timing.reset_us
            ));
            out.push_str(&format!("native_gates: {}\n", na.native_gates.join(", ")));
        }
    }

    out
}
