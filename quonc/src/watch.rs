//! Debounced filesystem watch loop for rapid experiment iteration.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use notify::event::{CreateKind, ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use quon_core::{
    ComparisonReport, MetricsSnapshot, RegressionConfig, compare, format_comparison_table,
    format_watch_metrics_line,
};

use crate::compile::{CompileReport, CompileRequest, compile};

/// Callback invoked after each successful watch-mode compile.
pub type WatchHandler =
    dyn FnMut(&CompileReport, Option<&MetricsSnapshot>, Option<&ComparisonReport>);

/// Debounce filesystem events until `debounce_ms` of quiet time elapses.
pub fn debounce_deadline(
    rx: &mpsc::Receiver<Result<notify::Event, notify::Error>>,
    debounce_ms: u64,
) -> Result<()> {
    let debounce = Duration::from_millis(debounce_ms);
    let mut deadline = Instant::now() + debounce;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(Ok(_event)) => {
                deadline = Instant::now() + debounce;
            }
            Ok(Err(err)) => return Err(err.into()),
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Disconnected) => bail!("watch channel closed"),
        }
    }
}

/// Returns true when `event` names one of `paths`.
pub fn event_targets_paths(event: &notify::Event, paths: &[PathBuf]) -> bool {
    let canonical: Vec<PathBuf> = paths.iter().filter_map(|p| p.canonicalize().ok()).collect();
    event.paths.iter().any(|event_path| {
        event_path
            .canonicalize()
            .ok()
            .is_some_and(|p| canonical.contains(&p))
    })
}

/// Locations watched for one input file.
///
/// The file itself stays in the list so events that name it still match
/// `watch_paths`. Its parent directory is included the same way as the entry
/// file: an editor rename-replace drops the old inode, and `is_relevant_event`
/// ignores the removal, so only a non-recursive watch on the parent sees the
/// new file.
pub(crate) fn input_watch_targets(path: &Path) -> Vec<PathBuf> {
    let mut targets = vec![path.to_path_buf()];
    if let Some(parent) = path.parent() {
        targets.push(parent.to_path_buf());
    }
    targets
}

/// Watch `path` and, when it has a parent, that parent non-recursively.
///
/// A parent-watch failure is ignored. The file watch is not: a missing input
/// should still abort `--watch`.
fn watch_file_and_parent(watcher: &mut RecommendedWatcher, path: &Path) -> Result<()> {
    for (index, target) in input_watch_targets(path).into_iter().enumerate() {
        let result = watcher.watch(&target, RecursiveMode::NonRecursive);
        if index == 0 {
            result.with_context(|| format!("watching {}", path.display()))?;
        } else {
            let _ = result;
        }
    }
    Ok(())
}

/// Returns true for modify/create/rename events we should react to.
pub fn is_relevant_event(event: &notify::Event) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Data(_))
            | EventKind::Modify(ModifyKind::Metadata(_))
            | EventKind::Modify(ModifyKind::Name(RenameMode::Any))
            | EventKind::Create(CreateKind::Any)
    )
}

/// Run the watch loop until interrupted or a fatal watcher error occurs.
pub fn run_watch_loop(
    source: PathBuf,
    extra_paths: &[PathBuf],
    target_path: Option<PathBuf>,
    debounce_ms: u64,
    make_request: impl Fn() -> Result<CompileRequest>,
    regression: Option<(PathBuf, RegressionConfig)>,
    mut on_success: impl FnMut(&CompileReport, Option<&MetricsSnapshot>, Option<&ComparisonReport>),
) -> Result<()> {
    let watch_paths: Vec<PathBuf> = std::iter::once(source.clone())
        .chain(extra_paths.iter().cloned())
        .chain(target_path.clone())
        .collect();

    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default(),
    )
    .context("creating filesystem watcher")?;

    watch_file_and_parent(&mut watcher, &source)?;
    for path in extra_paths {
        watch_file_and_parent(&mut watcher, path)?;
    }
    if let Some(ref target) = target_path {
        let _ = watcher.watch(target, RecursiveMode::NonRecursive);
    }

    let initial = make_request()?;
    let initial_report = compile(&initial);
    if initial_report.snapshot.compile.status == quon_core::CompileStatus::Ok {
        on_success(&initial_report, None, None);
    } else if let Some(err) = &initial_report.snapshot.compile.error {
        eprintln!("[quonc] compile error: {err}");
    }

    let mut previous: Option<MetricsSnapshot> = if initial_report.snapshot.metrics.is_some() {
        Some(initial_report.snapshot.clone())
    } else {
        None
    };
    let mut _sticky_regression = false;

    loop {
        match rx.recv() {
            Ok(Ok(event))
                if is_relevant_event(&event) && event_targets_paths(&event, &watch_paths) =>
            {
                debounce_deadline(&rx, debounce_ms)?;

                let request = make_request()?;
                let report = compile(&request);
                if report.snapshot.compile.status != quon_core::CompileStatus::Ok {
                    if let Some(err) = &report.snapshot.compile.error {
                        eprintln!("[quonc] compile error: {err}");
                    }
                    continue;
                }

                let comparison = if let Some((ref baseline_path, ref config)) = regression {
                    match quon_core::load_snapshot(baseline_path) {
                        Ok(baseline) => {
                            let report_cmp = compare(&baseline, &report.snapshot, config).ok();
                            if let Some(ref cmp) = report_cmp {
                                eprintln!(
                                    "{}",
                                    format_comparison_table(
                                        &baseline,
                                        &report.snapshot,
                                        cmp,
                                        config
                                    )
                                );
                                if !cmp.passed {
                                    _sticky_regression = true;
                                    eprintln!("[quonc] FAIL: regression detected");
                                }
                            }
                            report_cmp
                        }
                        Err(err) => {
                            eprintln!("[quonc] compare error: {err}");
                            None
                        }
                    }
                } else {
                    None
                };

                on_success(&report, previous.as_ref(), comparison.as_ref());
                previous = Some(report.snapshot.clone());
            }
            Ok(Ok(_)) => {}
            Ok(Err(err)) => return Err(err.into()),
            Err(_) => bail!("watch channel closed"),
        }
    }
}

/// Default watch success handler: metrics line with delta to stderr.
pub fn print_watch_metrics(
    report: &CompileReport,
    previous: Option<&MetricsSnapshot>,
    _comparison: Option<&ComparisonReport>,
) {
    eprintln!("{}", format_watch_metrics_line(&report.snapshot, previous));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_watch_targets_include_the_parent_directory() {
        let path = PathBuf::from("stdlib/qft.qn");
        assert_eq!(
            input_watch_targets(&path),
            vec![path, PathBuf::from("stdlib")]
        );
    }
}
