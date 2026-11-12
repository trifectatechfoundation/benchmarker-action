use indexmap::IndexMap;
use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::BufRead;
use std::process::Command;
use std::time::SystemTime;
use std::{env, fs};

use serde::{Deserialize, Serialize};

mod bench;
mod hardware;

use bench::*;

use crate::hardware::HardwareInformation;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Config {
    toolchains: IndexMap<String, Toolchain>,
    subjects: IndexMap<String, Subject>,
    benchmarks: IndexMap<String, Benchmark>,
    render_versus_self: Vec<Render>,
    render_versus_other: Vec<Render>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Toolchain {
    version: String,
    label: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Subject {
    name: String,
    version: String,
    toolchain: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Benchmark {
    #[serde(default)]
    repetitions: Option<u32>,
    commands: IndexMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Render {
    benchmark: String,
    measure: String,
    before: String,
    after: String,
    runs: Vec<Run>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Run {
    label: String,
    index: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchData {
    // When are we benchmarking
    commit_hash: String,
    commit_timestamp: u64,

    // timestamp when the benchmark was started
    timestamp: SystemTime,

    // Where are we benchmarking it on
    arch: String,
    os: String,
    runner: String,
    cpu_model: String,

    // What we are benchmarking
    toolchains: IndexMap<String, Toolchain>,
    subjects: IndexMap<String, Subject>,
    // The actual results for benchmarks
    benchmarks: IndexMap<String, IndexMap<String, Vec<SingleBench>>>,
}

impl Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        format_args!("{} {}", self.name, self.version).fmt(f)
    }
}

pub(crate) struct HumanReadable(f64);

impl Display for HumanReadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            1_000_000_000.0.. => write!(f, "{:6.2}G", self.0 / 1_000_000_000.0),
            1_000_000.0.. => write!(f, "{:6.2}M", self.0 / 1_000_000.0),
            1_000.0.. => write!(f, "{:6.2}K", self.0 / 1_000.0),
            _ => write!(f, "{:7.0}", self.0),
        }
    }
}

#[test]
fn human_readable() {
    assert_eq!(format!("{}", HumanReadable(12.0)), "     12");
    assert_eq!(format!("{}", HumanReadable(123.0)), "    123");
    assert_eq!(format!("{}", HumanReadable(1234.0)), "  1.23K");
    assert_eq!(format!("{}", HumanReadable(12345.0)), " 12.35K");
    assert_eq!(format!("{}", HumanReadable(123456.0)), "123.46K");
    assert_eq!(format!("{}", HumanReadable(1234567.0)), "  1.23M");
    assert_eq!(format!("{}", HumanReadable(1_000_000_000.0)), "  1.00G");
}

impl BenchData {
    /// The raw numbers for the commands. Good to have, but not the easiest to interpret
    fn render_markdown_raw(&self, md: &mut String, prev_results: Option<&Self>) {
        use std::fmt::Write;

        if let Some(prev_results) = prev_results {
            assert_eq!(self.arch, prev_results.arch);
            assert_eq!(self.os, prev_results.os);
            assert_eq!(self.runner, prev_results.runner);
            assert_eq!(self.cpu_model, prev_results.cpu_model);
        }

        // e.g. trifectatechfoundation/zlib-rs
        let repository = env::var("GITHUB_REPOSITORY").unwrap();

        if let Some(prev_results) = prev_results {
            writeln!(
                md,
                "## [`{commit}`](https://github.com/{repository}/commit/{commit}) with parent [`{commit_old}`](https://github.com/{repository}/commit/{commit_old}) \
                    (on {cpu})",
                commit = self.commit_hash,
                commit_old = prev_results.commit_hash,
                cpu = self.cpu_model
            )
                .unwrap();
        } else {
            writeln!(
                md,
                "## [`{commit}`](https://github.com/{repository}/commit/{commit}) \
                 (on {cpu})",
                commit = self.commit_hash,
                cpu = self.cpu_model
            )
            .unwrap();
        }
        writeln!(md, "").unwrap();

        for (bench_name, bench) in &self.benchmarks {
            let prev_bench_results = prev_results.and_then(|x| x.benchmarks.get(bench_name));

            writeln!(md, "### {}", bench_name).unwrap();
            writeln!(md).unwrap();

            for (subject, runs) in bench {
                writeln!(md, "#### {}", self.subjects[subject]).unwrap();
                writeln!(md).unwrap();

                let prev_runs = prev_bench_results.and_then(|x| {
                    x.iter().find_map(|(prev_subject, runs)| {
                        if subject == prev_subject {
                            Some(runs)
                        } else {
                            None
                        }
                    })
                });

                let mut available_counters = BTreeSet::new();
                for run in runs {
                    for counter in run.counters.keys() {
                        available_counters.insert(counter);
                    }
                }

                write!(md, "|command|").unwrap();
                for counter in &available_counters {
                    write!(md, "{counter}|{counter} Δ|").unwrap();
                }
                writeln!(md).unwrap();
                write!(md, "|---|").unwrap();
                for _ in &available_counters {
                    write!(md, "---|---|").unwrap();
                }
                writeln!(md).unwrap();

                for (idx, run) in runs.iter().enumerate() {
                    write!(md, "|`{}`|", run.cmd.join(" ")).unwrap();

                    let prev_run = prev_runs.and_then(|prev_runs| prev_runs.get(idx));

                    for &counter in &available_counters {
                        if let Some(data) = run.counters.get(counter) {
                            if let Some(prev_data) = prev_run.and_then(|prev_run| {
                                prev_run.counters.get(
                                    counter
                                        .strip_prefix("cpu_core/")
                                        .unwrap_or(counter)
                                        .strip_suffix("/")
                                        .unwrap_or(&counter),
                                )
                            }) {
                                let diff = if data.value > prev_data.value {
                                    format!(
                                        "+{:.1}%",
                                        (data.value - prev_data.value) as f64
                                            / prev_data.value as f64
                                            * 100.
                                    )
                                } else {
                                    format!(
                                        "-{:.1}%",
                                        (prev_data.value - data.value) as f64
                                            / prev_data.value as f64
                                            * 100.
                                    )
                                };

                                write!(
                                    md,
                                    "`{}±{}` {} | `{diff}` |",
                                    if data.unit == "msec" {
                                        format!("{:3.3}", data.value)
                                    } else {
                                        format!("{}", data.value)
                                    },
                                    data.variance.sqrt().round(),
                                    data.unit,
                                )
                                .unwrap();
                            } else {
                                write!(
                                    md,
                                    "`{}±{}` {} | `n.a.` |",
                                    if data.unit == "msec" {
                                        format!("{:3.3}", data.value)
                                    } else {
                                        format!("{}", data.value)
                                    },
                                    data.variance.sqrt().round(),
                                    data.unit,
                                )
                                .unwrap();
                            }
                        } else {
                            write!(md, "|").unwrap();
                        }
                    }
                    writeln!(md).unwrap();
                }
            }
        }
    }

    fn render_markdown_diff_pretty(
        md: &mut String,
        to_render: Vec<Render>,
        before: &Self,
        after: &Self,
    ) {
        use std::fmt::Write;

        assert_eq!(before.arch, after.arch);
        assert_eq!(before.os, after.os);
        assert_eq!(before.runner, after.runner);
        assert_eq!(before.cpu_model, after.cpu_model);

        // e.g. trifectatechfoundation/zlib-rs
        let repository = env::var("GITHUB_REPOSITORY").unwrap();

        writeln!(
            md,
            concat!(
                "## ",
                "[`{commit_new_short}`](https://github.com/{repository}/commit/{commit_new})",
                " with parent ",
                "[`{commit_old_short}`](https://github.com/{repository}/commit/{commit_old})",
                " (on {cpu})"
            ),
            repository = repository,
            commit_new = after.commit_hash,
            commit_old = before.commit_hash,
            commit_new_short = &after.commit_hash[..7],
            commit_old_short = &before.commit_hash[..7],
            cpu = after.cpu_model
        )
        .unwrap();

        for render in to_render {
            assert_eq!(render.before, render.after);

            writeln!(
                md,
                "### {} ({} vs self)",
                render.benchmark, before.subjects[&render.before].name
            )
            .unwrap();
            writeln!(md).unwrap();

            writeln!(md, "| name | [before](https://github.com/{repository}/commit/{commit_before}) | [after](https://github.com/{repository}/commit/{commit_after}) | Δ |",
                commit_before= before.commit_hash,
                commit_after= after.commit_hash,
            ).unwrap();

            writeln!(md, "| --- | --- | --- | --- |").unwrap();

            let Some(before_bench) = before.benchmarks.get(&render.benchmark) else {
                eprintln!(
                    "No results for benchmark {}, likely because it was newly added",
                    render.benchmark
                );
                continue;
            };
            let after_bench = &after.benchmarks[&render.benchmark];

            let Some(before_results) = before_bench.get(&render.before) else {
                eprintln!(
                    "No results for subject {}, likely because it was newly added",
                    render.before
                );
                continue;
            };
            let after_results = &after_bench[&render.after];

            for run in render.runs {
                let Some(before_result) = before_results.get(run.index) else {
                    continue;
                };
                let Some(after_result) = after_results.get(run.index) else {
                    continue;
                };
                let Some(before) = before_result.counters.get(&render.measure) else {
                    continue;
                };
                let Some(after) = after_result.counters.get(&render.measure) else {
                    continue;
                };

                BenchCounter::render_markdown_row(md, &run.label, before, after);
            }
        }
    }

    fn render_markdown_self_diff_pretty(md: &mut String, to_render: Vec<Render>, data: &Self) {
        use std::fmt::Write;

        // e.g. trifectatechfoundation/zlib-rs
        let repository = env::var("GITHUB_REPOSITORY").unwrap();

        writeln!(
            md,
            concat!(
                "## ",
                "[`{commit_new_short}`](https://github.com/{repository}/commit/{commit_new})",
                " (on {cpu})"
            ),
            repository = repository,
            commit_new = data.commit_hash,
            commit_new_short = &data.commit_hash[..7],
            cpu = data.cpu_model
        )
        .unwrap();

        for render in to_render {
            writeln!(
                md,
                "### {} ({} vs {})",
                render.benchmark,
                data.subjects[&render.before].name,
                data.subjects[&render.after].name
            )
            .unwrap();
            writeln!(md).unwrap();

            writeln!(md, "| name | before | after | Δ |",).unwrap();

            writeln!(md, "| --- | --- | --- | --- |").unwrap();

            let bench = &data.benchmarks[&render.benchmark];

            let Some(before_results) = bench.get(&render.before) else {
                eprintln!(
                    "No results for subject {}, likely because it was newly added",
                    render.before
                );
                continue;
            };
            let after_results = &bench[&render.after];

            for run in render.runs {
                let Some(before_result) = before_results.get(run.index) else {
                    continue;
                };
                let Some(after_result) = after_results.get(run.index) else {
                    continue;
                };
                let Some(before) = before_result.counters.get(&render.measure) else {
                    continue;
                };
                let Some(after) = after_result.counters.get(&render.measure) else {
                    continue;
                };

                BenchCounter::render_markdown_row(md, &run.label, before, after);
            }
        }
    }
}

fn run_in_shell(cmd: &str) -> String {
    String::from_utf8(
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string()
}

fn main() {
    let hardware = HardwareInformation::gather();
    let machine_name = hardware.machine_name();
    let best_cpu = hardware.best_benchmark_cpu();
    let cpu_model = hardware
        .cpu_model()
        .map(ToOwned::to_owned)
        .or(best_cpu.model())
        .unwrap_or_else(|| String::from("unknown"));

    eprintln!("Running on: {machine_name}");
    eprintln!("Will run benchmarks on CPU: {}", best_cpu.id());
    eprintln!("CPU Model: {cpu_model}");

    let mut args = env::args();
    let _ = args.next(); // ignore the path to the executable
                         //
    let commit_hash = args.next().unwrap();
    eprintln!("current commit: {}", commit_hash);

    let config_path = args.next().unwrap();
    let previous_results_path = args.next().unwrap();

    let commit_timestamp = {
        // git show 27b31a568651dd725488e422e854095639d75af6 --no-patch --pretty=format:"%ct"
        let output = Command::new("git")
            .args(&[
                "show",
                &commit_hash,
                "--no-patch",
                "--pretty=format:\"%ct\"",
            ])
            .output()
            .unwrap();

        String::from_utf8_lossy(&output.stdout)
            .trim()
            .trim_matches('"')
            .parse::<u64>()
            .unwrap()
    };

    let mut bench_data = BenchData {
        commit_hash,
        commit_timestamp,
        timestamp: SystemTime::now(),

        arch: env::var("RUNNER_ARCH").unwrap_or_else(|_| hardware.arch().to_string()),
        os: env::var("RUNNER_OS").unwrap_or_else(|_| hardware.os().to_string()),
        runner: env::var("RUNNER_NAME").unwrap_or_else(|_| hardware.hostname().to_string()),
        cpu_model: cpu_model.to_string(),

        toolchains: IndexMap::new(),
        subjects: IndexMap::new(),
        benchmarks: IndexMap::new(),
    };

    let config: Config = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();

    let prev_results = (|| {
        // we have two scenarios:
        //
        // - we benchmark on a PR merge into `main`
        // - we benchmark a commit versus current `main`
        let base_commit = String::from_utf8(
            Command::new("git")
                .arg("merge-base")
                .arg("origin/main")
                // Using HEAD~ rather than HEAD to get the parent commit if we are benchmarking for
                // the main branch.
                .arg("HEAD~")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();

        for line in fs::read(previous_results_path).unwrap_or_default().lines() {
            let Ok(data) = serde_json::from_str::<BenchData>(&line.unwrap()) else {
                continue; // Data format likely changed
            };

            if data.commit_hash == base_commit {
                return Some(data);
            }
        }

        None
    })();

    let base_commit_name = match prev_results {
        Some(ref prev_data) => prev_data.commit_hash.as_str(),
        None => "none",
    };
    eprintln!("base commit: {base_commit_name}",);

    for (toolchain_name, mut toolchain_info) in config.toolchains {
        toolchain_info.version = run_in_shell(&toolchain_info.version);

        bench_data.toolchains.insert(toolchain_name, toolchain_info);
    }

    for (subject_name, mut subject_info) in config.subjects {
        subject_info.version = run_in_shell(&subject_info.version);

        bench_data.subjects.insert(subject_name, subject_info);
    }

    for (bench_name, benchmark) in config.benchmarks {
        let mut bench_results = IndexMap::new();

        for (subject_name, commands) in benchmark.commands {
            let mut subject_results = vec![];

            for cmd in commands {
                subject_results.push(bench_single_cmd(
                    cmd.split(" ").map(|arg| arg.to_owned()).collect(),
                    benchmark.repetitions.unwrap_or(20),
                    best_cpu.cpu_set().clone(),
                ));
            }

            bench_results.insert(subject_name, subject_results);
        }

        bench_data.benchmarks.insert(bench_name, bench_results);
    }

    println!("{}", serde_json::to_string(&bench_data).unwrap());

    {
        let mut buf = String::new();
        bench_data.render_markdown_raw(&mut buf, prev_results.as_ref());
        eprintln!("{}", buf);
    }

    if let Ok(path) = env::var("GITHUB_STEP_SUMMARY") {
        let mut buf = String::new();

        if !config.render_versus_other.is_empty() {
            if let Some(prev_results) = prev_results.as_ref() {
                let converted = config.render_versus_other;

                BenchData::render_markdown_diff_pretty(
                    &mut buf,
                    converted,
                    prev_results,
                    &bench_data,
                );
            }
        }

        if !config.render_versus_self.is_empty() {
            BenchData::render_markdown_self_diff_pretty(
                &mut buf,
                config.render_versus_self,
                &bench_data,
            );
        }

        writeln!(buf, "### Toolchains").unwrap();
        writeln!(buf).unwrap();

        writeln!(buf, "|toolchain|version|").unwrap();
        writeln!(buf, "|---|----|").unwrap();
        for (_, toolchain) in &bench_data.toolchains {
            writeln!(buf, "|{}|{}|", toolchain.label, toolchain.version).unwrap();
        }
        writeln!(buf).unwrap();

        // hide the raw results if we're already showing some prettier tables
        let hide = !buf.is_empty();

        use std::fmt::Write;

        if hide {
            writeln!(buf, "<details>\n    <summary>Raw Results</summary>\n").unwrap();
        }

        bench_data.render_markdown_raw(&mut buf, prev_results.as_ref());

        if hide {
            writeln!(buf, "</details>").unwrap();
        }

        fs::write(&path, buf).unwrap();
    } else {
        eprintln!("The `GITHUB_STEP_SUMMARY` variable is not set, cannot write summary");
    }
}

#[test]
fn parse_render() {
    let input = r#"{ "label": "level 0", "index": 0 }"#;
    let _run: Run = serde_json::from_slice(input.as_bytes()).unwrap();

    let input = r#"
        {
            "benchmark": "compression",
            "measure": "cycles",
            "before": "zlib-ng-gcc",
            "after": "zlib-rs",
            "runs": [
                { "label": "level 0", "index": 0 },
                { "label": "level 1", "index": 1 },
                { "label": "level 2", "index": 2 },
                { "label": "level 3", "index": 3 },
                { "label": "level 4", "index": 4 },
                { "label": "level 5", "index": 5 },
                { "label": "level 6", "index": 6 },
                { "label": "level 7", "index": 7 },
                { "label": "level 8", "index": 8 },
                { "label": "level 9", "index": 9 }
            ]
        }
        "#;
    let _render: Render = serde_json::from_slice(input.as_bytes()).unwrap();

    let input = r#"
        [
            {
                "benchmark": "compression",
                "measure": "cycles",
                "before": "zlib-ng-gcc",
                "after": "zlib-rs",
                "runs": [
                    { "label": "level 0", "index": 0 },
                    { "label": "level 1", "index": 1 },
                    { "label": "level 2", "index": 2 },
                    { "label": "level 3", "index": 3 },
                    { "label": "level 4", "index": 4 },
                    { "label": "level 5", "index": 5 },
                    { "label": "level 6", "index": 6 },
                    { "label": "level 7", "index": 7 },
                    { "label": "level 8", "index": 8 },
                    { "label": "level 9", "index": 9 }
                ]
            },
            {
                "benchmark": "decompression",
                "measure": "cycles",
                "before": "zlib-ng-gcc",
                "after": "zlib-rs",
                "runs": [
                    { "label": "chunk size 4", "index": 0 },
                    { "label": "chunk size 5", "index": 1 },
                    { "label": "chunk size 6", "index": 2 },
                    { "label": "chunk size 7", "index": 3 },
                    { "label": "chunk size 8", "index": 4 },
                    { "label": "chunk size 9", "index": 5 },
                    { "label": "chunk size 10", "index": 6 },
                    { "label": "chunk size 11", "index": 7 },
                    { "label": "chunk size 12", "index": 8 },
                    { "label": "chunk size 13", "index": 9 },
                    { "label": "chunk size 14", "index": 10 },
                    { "label": "chunk size 15", "index": 11 },
                    { "label": "chunk size 16", "index": 12 }
                ]
            }
        ]
    "#;

    let _renders: Vec<Render> = serde_json::from_slice(input.as_bytes()).unwrap();
}
