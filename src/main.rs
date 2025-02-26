use indexmap::IndexMap;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Display;
use std::io::BufRead;
use std::process::Command;
use std::time::SystemTime;
use std::{env, fs};

use serde::{Deserialize, Serialize};

mod bench;

use bench::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Config {
    #[serde(default)]
    repetitions_for_group: HashMap<String, u32>,
    commands: IndexMap<String, Vec<String>>,
    render_versus_self: IndexMap<String, IndexMap<String, Compare>>,
    render_versus_other: IndexMap<String, VersusOther>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct VersusOther {
    measure: String,
    command: String,
    rows: IndexMap<String, usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Compare {
    measure: String,
    before: Reference,
    after: Reference,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Reference {
    command: String,
    index: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct BenchData {
    // What and when are we benchmarking
    commit_hash: String,
    commit_timestamp: u64,

    // timestamp when the benchmark was started
    timestamp: SystemTime,

    // Where are we benchmarking it on
    arch: String,
    os: String,
    runner: String,
    cpu_model: String,

    // The actual results for benchmarks
    bench_groups: IndexMap<String, Vec<SingleBench>>,
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

        for (group_name, group_results) in &self.bench_groups {
            let prev_group_results = prev_results.and_then(|x| x.bench_groups.get(group_name));

            writeln!(md, "### {}", group_name).unwrap();
            writeln!(md).unwrap();

            let mut available_counters = BTreeSet::new();
            for bench in group_results {
                for counter in bench.counters.keys() {
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

            for bench in group_results {
                let prev_bench = prev_group_results
                    .and_then(|x| x.iter().find(|prev_bench| prev_bench.cmd == bench.cmd));

                write!(md, "|`{}`|", bench.cmd.join(" ")).unwrap();

                for &counter in &available_counters {
                    if let Some(data) = bench.counters.get(counter) {
                        if let Some(prev_data) = prev_bench.and_then(|prev_bench| {
                            prev_bench.counters.get(
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
                                    (data.value - prev_data.value) as f64 / prev_data.value as f64
                                        * 100.
                                )
                            } else {
                                format!(
                                    "-{:.1}%",
                                    (prev_data.value - data.value) as f64 / prev_data.value as f64
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

    fn render_markdown_diff_pretty(
        md: &mut String,
        render: IndexMap<String, VersusOther>,
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

        for (group_name, rows) in render {
            writeln!(md, "### {group_name}").unwrap();
            writeln!(md).unwrap();

            writeln!(md, "| name | [before](https://github.com/{repository}/commit/{commit_before}) | [after](https://github.com/{repository}/commit/{commit_after}) | Δ |",
                commit_before= before.commit_hash,
                commit_after= after.commit_hash,
            ).unwrap();

            writeln!(md, "| --- | --- | --- | --- |").unwrap();

            for (name, row) in rows.rows {
                let Some(before) = &before.bench_groups[&rows.command][row]
                    .counters
                    .get(&rows.measure)
                else {
                    continue;
                };
                let Some(after) = &after.bench_groups[&rows.command][row]
                    .counters
                    .get(&rows.measure)
                else {
                    continue;
                };

                BenchCounter::render_markdown_row(md, &name, before, after);
            }
        }
    }

    fn render_markdown_self_diff_pretty(
        md: &mut String,
        render: IndexMap<String, IndexMap<String, Compare>>,
        data: &Self,
    ) {
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

        for (group_name, rows) in render {
            writeln!(md, "### {group_name}").unwrap();
            writeln!(md).unwrap();

            writeln!(md, "| name | before | after | Δ |",).unwrap();

            writeln!(md, "| --- | --- | --- | --- |").unwrap();

            for (name, row) in rows {
                let Some(before) = &data.bench_groups[&row.before.command][row.before.index]
                    .counters
                    .get(&row.measure)
                else {
                    continue;
                };
                let Some(after) = &data.bench_groups[&row.after.command][row.after.index]
                    .counters
                    .get(&row.measure)
                else {
                    continue;
                };

                BenchCounter::render_markdown_row(md, &name, before, after);
            }
        }
    }
}

fn get_cpu_model() -> String {
    if cfg!(target_os = "linux") {
        serde_json::from_slice::<serde_json::Value>(
            &Command::new("lscpu").arg("-J").output().unwrap().stdout,
        )
        .unwrap()["lscpu"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["field"] == "Model name:")
            .unwrap()["data"]
            .as_str()
            .unwrap()
            .to_owned()
    } else if cfg!(target_os = "macos") {
        String::from_utf8(
            Command::new("sysctl")
                .arg("-n")
                .arg("machdep.cpu.brand_string")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned()
    } else {
        "unknown".to_owned()
    }
}

fn main() {
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

        arch: env::var("RUNNER_ARCH").unwrap_or_default(),
        os: env::var("RUNNER_OS").unwrap_or_default(),
        runner: env::var("RUNNER_NAME").unwrap_or_else(|_| "<local bench>".to_owned()),
        cpu_model: get_cpu_model(),

        bench_groups: IndexMap::new(),
    };

    let config: Config = serde_json::from_slice(&fs::read(config_path).unwrap()).unwrap();

    let commands = config.commands;

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

    for (group_name, benches) in commands {
        let mut group_results = vec![];
        for cmd in benches {
            group_results.push(bench_single_cmd(
                cmd.split(" ").map(|arg| arg.to_owned()).collect(),
                config
                    .repetitions_for_group
                    .get(&group_name)
                    .copied()
                    .unwrap_or(20),
            ));
        }
        bench_data.bench_groups.insert(group_name, group_results);
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
                    &prev_results,
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
    }
}

#[test]
fn parse_render() {
    let input = r#"{ "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 0 }, "after": { "command": "blogpost-compress-rs", "index": 0 } }"#;
    let _compare: Compare = serde_json::from_slice(input.as_bytes()).unwrap();

    let input = r#"
        {
            "level 0": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 0 }, "after": { "command": "blogpost-compress-rs", "index": 0 } },
            "level 1": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 1 }, "after": { "command": "blogpost-compress-rs", "index": 1 } },
            "level 2": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 2 }, "after": { "command": "blogpost-compress-rs", "index": 2 } },
            "level 3": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 3 }, "after": { "command": "blogpost-compress-rs", "index": 3 } },
            "level 4": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 4 }, "after": { "command": "blogpost-compress-rs", "index": 4 } },
            "level 5": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 5 }, "after": { "command": "blogpost-compress-rs", "index": 5 } },
            "level 6": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 6 }, "after": { "command": "blogpost-compress-rs", "index": 6 } },
            "level 7": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 7 }, "after": { "command": "blogpost-compress-rs", "index": 7 } },
            "level 8": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 8 }, "after": { "command": "blogpost-compress-rs", "index": 8 } },
            "level 9": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 9 }, "after": { "command": "blogpost-compress-rs", "index": 9 } }
        }
        "#;
    let _compares: IndexMap<String, Compare> = serde_json::from_slice(input.as_bytes()).unwrap();

    let input = r#"
        {
            "compression (ng vs rs)": {
                "level 0": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 0 }, "after": { "command": "blogpost-compress-rs", "index": 0 } },
                "level 1": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 1 }, "after": { "command": "blogpost-compress-rs", "index": 1 } },
                "level 2": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 2 }, "after": { "command": "blogpost-compress-rs", "index": 2 } },
                "level 3": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 3 }, "after": { "command": "blogpost-compress-rs", "index": 3 } },
                "level 4": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 4 }, "after": { "command": "blogpost-compress-rs", "index": 4 } },
                "level 5": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 5 }, "after": { "command": "blogpost-compress-rs", "index": 5 } },
                "level 6": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 6 }, "after": { "command": "blogpost-compress-rs", "index": 6 } },
                "level 7": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 7 }, "after": { "command": "blogpost-compress-rs", "index": 7 } },
                "level 8": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 8 }, "after": { "command": "blogpost-compress-rs", "index": 8 } },
                "level 9": { "measure": "cycles", "before": { "command": "blogpost-compress-ng", "index": 9 }, "after": { "command": "blogpost-compress-rs", "index": 9 } }
            },
            "decompression (ng vs rs)": {
                "chunk size 4": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 0 }, "after": { "command": "blogpost-uncompress-rs", "index": 0 } },
                "chunk size 5": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 1 }, "after": { "command": "blogpost-uncompress-rs", "index": 1 } },
                "chunk size 6": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 2 }, "after": { "command": "blogpost-uncompress-rs", "index": 2 } },
                "chunk size 7": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 3 }, "after": { "command": "blogpost-uncompress-rs", "index": 3 } },
                "chunk size 8": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 4 }, "after": { "command": "blogpost-uncompress-rs", "index": 4 } },
                "chunk size 9": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 5 }, "after": { "command": "blogpost-uncompress-rs", "index": 5 } },
                "chunk size 10": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 6 }, "after": { "command": "blogpost-uncompress-rs", "index": 6 } },
                "chunk size 11": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 7 }, "after": { "command": "blogpost-uncompress-rs", "index": 7 } },
                "chunk size 12": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 8 }, "after": { "command": "blogpost-uncompress-rs", "index": 8 } },
                "chunk size 13": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 9 }, "after": { "command": "blogpost-uncompress-rs", "index": 9 } },
                "chunk size 14": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 10 }, "after": { "command": "blogpost-uncompress-rs", "index": 10 } },
                "chunk size 15": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 11 }, "after": { "command": "blogpost-uncompress-rs", "index": 11 } },
                "chunk size 16": { "measure": "cycles", "before": { "command": "blogpost-uncompress-ng", "index": 12 }, "after": { "command": "blogpost-uncompress-rs", "index": 12 } }
            }
        }
    "#;

    let _render: IndexMap<String, IndexMap<String, Compare>> =
        serde_json::from_slice(input.as_bytes()).unwrap();
}
