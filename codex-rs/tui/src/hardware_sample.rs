use super::Metrics;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

#[derive(Default)]
pub(super) struct State {
    #[cfg(target_os = "linux")]
    previous_cpu: Option<(u64, u64)>,
    #[cfg(target_os = "macos")]
    smc: Option<super::smc::Smc>,
}

async fn command(program: &str, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(super) fn percentage(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && (0.0..=100.0).contains(n))
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn mac_cpu(text: &str) -> Option<f64> {
    let line = text.lines().rfind(|line| line.starts_with("CPU usage:"))?;
    let idle = line.rsplit(',').next()?.trim().strip_suffix("% idle")?;
    percentage(idle).map(|idle| 100.0 - idle)
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn mac_gpu(text: &str) -> Option<f64> {
    let value = text.split_once("\"Device Utilization %\"=")?.1;
    percentage(value.split([',', '}']).next()?)
}

#[cfg(any(target_os = "macos", test))]
fn memory_size(text: &str) -> Option<u64> {
    let suffix = text.chars().last()?;
    let value = text[..text.len() - 1].parse::<u64>().ok()?;
    let scale = match suffix {
        'G' => 1_073_741_824,
        'M' => 1_048_576,
        'K' => 1024,
        'B' => 1,
        _ => return None,
    };
    value.checked_mul(scale)
}

#[cfg(target_os = "macos")]
pub(super) async fn read(state: &mut State) -> Metrics {
    let smc = state.smc.take();
    let temperatures = tokio::task::spawn_blocking(move || {
        let smc = smc.or_else(super::smc::Smc::open);
        let temperatures = smc
            .as_ref()
            .map(super::smc::Smc::temperatures)
            .unwrap_or_default();
        (smc, temperatures)
    });
    let (top, gpu, name, total, temperatures) = tokio::join!(
        command("/usr/bin/top", &["-l", "2", "-n", "0", "-s", "1"]),
        command(
            "/usr/sbin/ioreg",
            &["-r", "-d", "1", "-c", "AGXAccelerator"]
        ),
        command("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"]),
        command("/usr/sbin/sysctl", &["-n", "hw.memsize"]),
        temperatures,
    );
    let (smc, (cpu_temperature, gpu_temperature)) = temperatures.unwrap_or_default();
    state.smc = smc;
    let top = top.unwrap_or_default();
    let gpu = gpu.unwrap_or_default();
    let used = top
        .lines()
        .filter_map(|line| line.strip_prefix("PhysMem: "))
        .next_back()
        .and_then(|line| memory_size(line.split_whitespace().next()?));
    let total = total.and_then(|value| value.trim().parse().ok());
    let gpu_name = gpu
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("\"model\" = \"")
                .and_then(|value| value.strip_suffix('"'))
        })
        .unwrap_or("Apple GPU")
        .to_owned();
    Metrics {
        cpu_name: name.unwrap_or_else(|| "CPU".into()).trim().to_owned(),
        cpu: mac_cpu(&top),
        ram: used.zip(total),
        gpu_name,
        gpu: mac_gpu(&gpu),
        cpu_temperature,
        gpu_temperature,
    }
}

#[cfg(target_os = "linux")]
pub(super) async fn read(state: &mut State) -> Metrics {
    let previous = &mut state.previous_cpu;
    let mut metrics = Metrics::default();
    if let Ok(text) = tokio::fs::read_to_string("/proc/stat").await {
        let ticks: Vec<u64> = text
            .lines()
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .skip(1)
            .take(8)
            .filter_map(|value| value.parse().ok())
            .collect();
        if ticks.len() >= 4 {
            let total = ticks.iter().sum::<u64>();
            let idle = ticks[3] + ticks.get(4).copied().unwrap_or_default();
            if let Some((old_total, old_idle)) = previous.replace((total, idle)) {
                let elapsed = total.saturating_sub(old_total);
                if elapsed > 0 {
                    metrics.cpu = Some(
                        100.0 * elapsed.saturating_sub(idle.saturating_sub(old_idle)) as f64
                            / elapsed as f64,
                    );
                }
            }
        }
    }
    if let Ok(text) = tokio::fs::read_to_string("/proc/cpuinfo").await {
        metrics.cpu_name = text
            .lines()
            .find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, name)| name.trim().to_owned())
            })
            .unwrap_or_else(|| "CPU".into());
    }
    if let Ok(text) = tokio::fs::read_to_string("/proc/meminfo").await {
        let kb = |key| {
            text.lines()
                .find_map(|line| line.strip_prefix(key))
                .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
                .map(|n| n * 1024)
        };
        metrics.ram = kb("MemTotal:")
            .zip(kb("MemAvailable:"))
            .map(|(total, available)| (total.saturating_sub(available), total));
    }
    if let Some(text) = command(
        "nvidia-smi",
        &[
            "--query-gpu=name,utilization.gpu,temperature.gpu",
            "--format=csv,noheader,nounits",
            "--id=0",
        ],
    )
    .await
    {
        let fields: Vec<_> = text.trim().split(',').collect();
        if fields.len() == 3 {
            metrics.gpu_name = fields[0].trim().to_owned();
            metrics.gpu = percentage(fields[1]);
            metrics.gpu_temperature = fields[2]
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite() && (0.0..=150.0).contains(n));
        }
    }
    if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/thermal").await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let kind = tokio::fs::read_to_string(entry.path().join("type"))
                .await
                .unwrap_or_default();
            if matches!(kind.trim(), "x86_pkg_temp" | "cpu-thermal") {
                metrics.cpu_temperature = tokio::fs::read_to_string(entry.path().join("temp"))
                    .await
                    .ok()
                    .and_then(|value| value.trim().parse::<f64>().ok())
                    .map(|n| n / 1000.0)
                    .filter(|n| n.is_finite() && (0.0..=150.0).contains(n));
                break;
            }
        }
    }
    metrics
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) async fn read(_state: &mut State) -> Metrics {
    Metrics::default()
}
