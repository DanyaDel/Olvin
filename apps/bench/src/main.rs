//! The benchmark suite: what the engine's hot paths cost on reference scenes.
//!
//! ```text
//! cargo run --release -p olvin_bench -- [--list] [--only text] [--json out.json]
//!     [--compare baseline.json] [--tolerance 3.0] [--scale 0.5]
//! ```
//!
//! Each case prints its median and best time in milliseconds. `--json` writes
//! them for CI to keep; `--compare` reports every case slower than the
//! baseline by more than `--tolerance` times and exits with 1 (runners differ
//! a lot, so the tolerance is for order-of-magnitude regressions: compare
//! numbers across commits on one machine for anything finer). Cases that need
//! a GPU are skipped without an adapter (CI uses Mesa's llvmpipe).

mod cases;

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

/// One measured case.
struct Outcome {
    name: &'static str,
    median_ms: f64,
    best_ms: f64,
    runs: usize,
}

/// A case: its name, what it measures, and how to run it. `scale` shrinks
/// the number of measured runs (never below 3).
pub struct Case {
    pub name: &'static str,
    pub what: &'static str,
    pub run: fn(scale: f64) -> Option<Vec<f64>>,
}

/// Times `f` `runs` times after `warmup` unmeasured calls, in milliseconds.
pub fn measure(warmup: usize, runs: usize, scale: f64, mut f: impl FnMut()) -> Vec<f64> {
    for _ in 0..warmup {
        f();
    }
    let runs = ((runs as f64 * scale).round() as usize).max(3);
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect()
}

/// Like [`measure`], with an untimed `setup` before each run.
pub fn measure_with<S>(
    runs: usize,
    scale: f64,
    mut setup: impl FnMut() -> S,
    mut f: impl FnMut(S),
) -> Vec<f64> {
    let runs = ((runs as f64 * scale).round() as usize).max(3);
    (0..runs)
        .map(|_| {
            let state = setup();
            let t = Instant::now();
            f(state);
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect()
}

fn summarize(name: &'static str, mut samples: Vec<f64>) -> Outcome {
    samples.sort_by(f64::total_cmp);
    Outcome {
        name,
        median_ms: samples[samples.len() / 2],
        best_ms: samples[0],
        runs: samples.len(),
    }
}

fn to_json(results: &[Outcome]) -> String {
    let cases: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "median_ms": r.median_ms,
                "best_ms": r.best_ms,
                "runs": r.runs,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cases": cases,
    }))
    .unwrap_or_default()
}

/// `name → median_ms` of a file written by `--json`.
fn read_baseline(path: &str) -> std::result::Result<BTreeMap<String, f64>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
    let cases = value["cases"].as_array().ok_or("no `cases` list")?;
    Ok(cases
        .iter()
        .filter_map(|c| Some((c["name"].as_str()?.to_owned(), c["median_ms"].as_f64()?)))
        .collect())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let (mut only, mut json, mut compare) = (None, None, None);
    let (mut tolerance, mut scale, mut list) = (3.0f64, 1.0f64, false);
    while let Some(arg) = args.next() {
        let mut value = |what: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("{what} needs a value");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--list" => list = true,
            "--only" => only = Some(value("--only")),
            "--json" => json = Some(value("--json")),
            "--compare" => compare = Some(value("--compare")),
            "--tolerance" => tolerance = value("--tolerance").parse().unwrap_or(3.0),
            "--scale" => scale = value("--scale").parse().unwrap_or(1.0),
            other => {
                eprintln!("unknown argument `{other}`");
                return ExitCode::from(2);
            }
        }
    }
    let all = cases::all();
    if list {
        for c in &all {
            println!("{:<34} {}", c.name, c.what);
        }
        return ExitCode::SUCCESS;
    }
    let mut results = Vec::new();
    println!(
        "{:<34} {:>11} {:>11} {:>5}",
        "case", "median ms", "best ms", "runs"
    );
    for case in all
        .iter()
        .filter(|c| only.as_ref().is_none_or(|o| c.name.contains(o.as_str())))
    {
        match (case.run)(scale) {
            Some(samples) => {
                let r = summarize(case.name, samples);
                println!(
                    "{:<34} {:>11.3} {:>11.3} {:>5}",
                    r.name, r.median_ms, r.best_ms, r.runs
                );
                results.push(r);
            }
            None => println!("{:<34} {:>11}", case.name, "skipped"),
        }
    }
    if let Some(path) = json
        && let Err(e) = std::fs::write(&path, to_json(&results))
    {
        eprintln!("{path}: {e}");
        return ExitCode::from(2);
    }
    let Some(path) = compare else {
        return ExitCode::SUCCESS;
    };
    let baseline = match read_baseline(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("baseline: {e}");
            return ExitCode::from(2);
        }
    };
    let mut slower = 0;
    for r in &results {
        let Some(&base) = baseline.get(r.name) else {
            println!("{}: not in the baseline", r.name);
            continue;
        };
        let ratio = r.median_ms / base.max(1e-6);
        if ratio > tolerance {
            slower += 1;
            println!(
                "SLOWER {}: {:.3} ms against {:.3} ms ({ratio:.1}x, tolerance {tolerance}x)",
                r.name, r.median_ms, base
            );
        }
    }
    if slower > 0 {
        ExitCode::FAILURE
    } else {
        println!("no case slower than {tolerance}x its baseline");
        ExitCode::SUCCESS
    }
}
