use std::collections::BTreeMap;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct SingleBench {
    pub cmd: Vec<String>,
    pub counters: BTreeMap<String, BenchCounter>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BenchCounter {
    pub value: f64,
    pub variance: f64,
    pub unit: String,
}

impl BenchCounter {
    pub fn improvement_percentage(old: &Self, new: &Self) -> f64 {
        ((old.value - new.value) / old.value) * 100.0
    }

    pub fn is_significant(old: &Self, new: &Self, repetitions: u32) -> bool {
        let f = old;
        let m = new;

        let half = {
            let z = get_stat_score_95(2 * repetitions - 2);
            let n1: f64 = repetitions as f64;
            let n2: f64 = repetitions as f64;
            let normer = (1.0 / n1 + 1.0 / n2).sqrt();
            let numer1 = (n1 - 1.0) * m.variance;
            let numer2 = (n2 - 1.0) * f.variance;
            let df = n1 + n2 - 2.0;
            let sp = ((numer1 + numer2) / df).sqrt();
            (z * sp * normer) * 100.0 / f.value
        };

        let diff_mean_percent = (m.value - f.value) * 100.0 / f.value;

        // significant only if full interval is beyond abs 1% with the same sign
        if diff_mean_percent >= 1.0 && (diff_mean_percent - half) >= 1.0 {
            true
        } else if diff_mean_percent <= -1.0 && (diff_mean_percent + half) <= -1.0 {
            true
        } else {
            false
        }
    }
}

pub fn bench_single_cmd(cmd: Vec<String>) -> SingleBench {
    // FIXME show some progress notification
    if cfg!(target_os = "linux") {
        bench_single_cmd_perf(cmd)
    } else {
        bench_single_cmd_getrusage(cmd)
    }
}

fn bench_single_cmd_perf(cmd: Vec<String>) -> SingleBench {
    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct PerfData {
        event: String,
        counter_value: String,
        unit: String,
        variance: f64,
    }

    let mut perf_stat_cmd = Command::new("perf");
    perf_stat_cmd
        // Perf produces broken JSON when the system locale uses decimal comma rather than decimal point.
        .env("LANG", "C")
        .arg("stat")
        .arg("-j")
        .arg("-e")
        .arg("task-clock,cycles,instructions")
        .arg("--repeat")
        .arg("20")
        .arg("--");
    perf_stat_cmd.args(&cmd);

    let output = perf_stat_cmd.output().unwrap();
    assert!(
        output.status.success(),
        "`{:?}` failed with {:?}:=== stdout ===\n{}\n\n=== stderr ===\n{}",
        perf_stat_cmd,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let counters = String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| {
            serde_json::from_str::<PerfData>(line)
                .unwrap_or_else(|e| panic!("Failed to parse {line:?}: {e}"))
        })
        .filter(|counter| counter.counter_value != "<not counted>")
        .map(|counter| {
            (
                counter.event,
                BenchCounter {
                    value: counter
                        .counter_value
                        .parse::<f64>()
                        .unwrap_or_else(|_| panic!("Failed to parse {}", counter.counter_value)),
                    variance: counter.variance,
                    unit: counter.unit,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    SingleBench { cmd, counters }
}

fn bench_single_cmd_getrusage(cmd: Vec<String>) -> SingleBench {
    use std::mem;
    use std::time::Duration;

    fn get_cpu_times() -> Duration {
        use libc::{getrusage, rusage, RUSAGE_CHILDREN};

        let result: rusage = unsafe {
            let mut buf = mem::zeroed();
            let success = getrusage(RUSAGE_CHILDREN, &mut buf);
            assert_eq!(0, success);
            buf
        };

        Duration::new(
            result.ru_utime.tv_sec as _,
            (result.ru_utime.tv_usec * 1000) as _,
        )
    }

    let mut bench_cmd = Command::new(cmd.get(0).unwrap());
    bench_cmd.args(&cmd[1..]);

    let start_cpu = get_cpu_times();
    let output = bench_cmd.output().unwrap();
    let user_time = get_cpu_times() - start_cpu;
    assert!(
        output.status.success(),
        "`{:?}` failed with {:?}:\n=== stdout ===\n{}\n\n=== stderr ===\n{}",
        bench_cmd,
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    SingleBench {
        cmd,
        counters: BTreeMap::from_iter([(
            "user-time".to_owned(),
            BenchCounter {
                value: user_time.as_secs_f64() * 1000.0,
                unit: "msec".to_owned(),
                variance: 0.0,
            },
        )]),
    }
}

// Gets either the T or Z score for 95% confidence.
// If no `df` variable is provided, Z score is provided.
fn get_stat_score_95(df: u32) -> f64 {
    let dfv: usize = df as usize;
    if dfv <= 30 {
        return T_TABLE95_1TO30[dfv - 1];
    } else if dfv <= 120 {
        let idx_10s = dfv / 10;
        return T_TABLE95_10S_10TO120[idx_10s - 1];
    }

    return 1.96;
}

const T_TABLE95_1TO30: [f64; 30] = [
    12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.16,
    2.145, 2.131, 2.12, 2.11, 2.101, 2.093, 2.086, 2.08, 2.074, 2.069, 2.064, 2.06, 2.056, 2.052,
    2.045, 2.048, 2.042,
];

const T_TABLE95_10S_10TO120: [f64; 12] = [
    2.228, 2.086, 2.042, 2.021, 2.009, 2.0, 1.994, 1.99, 1.987, 1.984, 1.982, 1.98,
];
