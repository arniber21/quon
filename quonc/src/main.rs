use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anstyle::{AnsiColor, Color, Style};
use anyhow::{Context as _, Result, anyhow, bail};
use clap::{ArgAction, ColorChoice, Parser, ValueEnum};
use quon_core::{
    RegressionConfig, compare, format_comparison_table, format_metrics_line, load_snapshot,
    save_snapshot,
};
use quon_na::{
    PlacementStrategy, PlacerMode, resource_report_to_json, resource_report_to_markdown,
};

use backend::{BackendTarget, TargetKind};
use quonc::compile::{CompileRequest, compile, schedule_to_json};
use quonc::na_target::{NaBackendKind, parse_na_backend, parse_placer_mode};
use quonc::watch::{print_watch_metrics, run_watch_loop};

const AFTER_HELP: &str = "\
Examples:
  # Fixed / OpenQASM path
  quonc program.qn --emit-qasm
  quonc program.qn --target targets/ibm/fake_manila.json --emit-qasm --metrics

  # Neutral-atom path (#112)
  quonc program.qn --target targets/neutral_atom/generic_rna_v0.json \\
    --emit-na-schedule --emit-resource-report

  # Debug IR after each pass
  quonc program.qn --dump-ir --verify-linear --emit-qasm

  # Inspect a target without compiling
  quonc --target targets/neutral_atom/generic_rna_v0.json --print-target

Notes:
  Fixed targets run SABRE routing and emit OpenQASM 3.0.
  Neutral-atom targets extract an interaction graph, schedule entangling
  layers, run zoned RAP (default) or flat AOD movement, optionally compact,
  then emit schedule JSON and/or a resource report.
";

#[derive(Parser, Debug)]
#[command(
    name = "quonc",
    about = "Quon quantum compiler",
    long_about = "Quon quantum compiler — OpenQASM 3.0 and neutral-atom schedules.\n\n\
Compile Quon programs through the MLIR pipeline. Fixed (gate-model) targets \
emit OpenQASM 3.0. Neutral-atom reconfigurable targets schedule AOD movement \
/ zoned RAP and emit schedule JSON plus resource reports.",
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
    /// Source file to compile (.qn). Optional with --print-target / --list-passes.
    source: Option<PathBuf>,

    // ── Emit ────────────────────────────────────────────────────────────
    /// Emit OpenQASM 3.0 (fixed targets only)
    #[arg(long, help_heading = "Emit", action = ArgAction::SetTrue)]
    emit_qasm: bool,

    /// Emit neutral-atom schedule JSON (`-` = stdout)
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_na_schedule: Option<String>,

    /// Emit neutral-atom resource report (`-` = stdout; `.md` → Markdown, else JSON)
    #[arg(
        long,
        value_name = "PATH",
        num_args = 0..=1,
        default_missing_value = "-",
        help_heading = "Emit"
    )]
    emit_resource_report: Option<String>,

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

    /// Zoned placer mode: routing-agnostic (default) or routing-aware
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

    // ── Debug ───────────────────────────────────────────────────────────
    /// Dump MLIR after each major pass stage to stderr
    #[arg(long, help_heading = "Debug", action = ArgAction::SetTrue)]
    dump_ir: bool,

    /// Run circ/dynamic linearity verifiers (debug)
    #[arg(long, help_heading = "Debug", action = ArgAction::SetTrue)]
    verify_linear: bool,

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

    emit_artifacts(&cli, &report)?;

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
             use --emit-na-schedule / --emit-resource-report for neutral-atom targets"
        );
    }
    if (cli.emit_na_schedule.is_some() || cli.emit_resource_report.is_some()) && !is_na {
        bail!(
            "--emit-na-schedule / --emit-resource-report require a \
             neutral_atom_reconfigurable target (see targets/neutral_atom/)"
        );
    }
    Ok(())
}

fn emit_artifacts(cli: &Cli, report: &quonc::CompileReport) -> Result<()> {
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

    if let Some(path) = &cli.emit_na_schedule {
        let layers = report.na_schedule.as_ref().ok_or_else(|| {
            anyhow!("no NA schedule available (compile with a neutral-atom target)")
        })?;
        let json = schedule_to_json(layers)?;
        write_output(path, &json, qasm_owns_stdout && path == "-")?;
        emitted = true;
    }

    if let Some(path) = &cli.emit_resource_report {
        let report_body = report.resource_report.as_ref().ok_or_else(|| {
            anyhow!("no resource report available (compile with a neutral-atom target)")
        })?;
        let text = match resolve_report_format(cli, path) {
            ReportFormat::Json => resource_report_to_json(report_body)?,
            ReportFormat::Markdown => resource_report_to_markdown(report_body),
        };
        // If schedule already printed to stdout on `-`, send the report to stderr
        // so both artifacts remain recoverable without interleaving JSON values.
        let schedule_on_stdout = cli.emit_na_schedule.as_ref().is_some_and(|p| p == "-");
        write_output(
            path,
            &text,
            (qasm_owns_stdout || schedule_on_stdout) && path == "-",
        )?;
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
                     pass --emit-na-schedule / --emit-resource-report){dim:#}"
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
  1. lower            Quon → quantum.circ
  2. circ fixpoint    gate_cancellation, rotation_merging,
                      compiler_uncomputation, zx_simplification, clifford_t_opt
  3. monadic_lowering quantum.circ → quantum.dynamic
  4. dynamic          measurement_deferral, classical_region_fusion

Fixed (OpenQASM) path
  5. native_gate_decomp
  6. sabre_routing
  7. native_gate_decomp (post-SWAP)
  8. depth_scheduling
  9. emit OpenQASM 3.0

Neutral-atom path
  5. extract_interaction_graph
  6. schedule_entangling_layers (Misra–Gries / ASAP)
  7. schedule_zoned (default)  OR  place + plan_aod_movement (--na-backend flat)
  8. compact_schedule (unless --no-na-compact)
  9. build_resource_report / emit schedule JSON
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

    Ok(CompileRequest {
        source_path,
        source,
        target,
        target_descriptor_path: cli.target.clone(),
        dump_ir: cli.dump_ir,
        verify_linear: cli.verify_linear,
        sabre_gamma: cli.sabre_gamma,
        na_backend: cli.na_backend,
        na_placer: cli.na_placer,
        na_compact: !cli.no_na_compact,
        na_placement: cli.na_placement.into(),
    })
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
