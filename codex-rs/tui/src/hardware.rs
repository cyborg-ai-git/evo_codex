//! Read-only machine telemetry, sampled away from the rendering/event loop.
use crate::tui::FrameRequester;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::layout::Size;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

#[path = "hardware_sample.rs"]
mod sample;

#[cfg(target_os = "macos")]
#[path = "hardware_smc.rs"]
mod smc;

#[derive(Clone, Debug, Default)]
pub(super) struct Metrics {
    cpu_name: String,
    cpu: Option<f64>,
    ram: Option<(u64, u64)>,
    gpu_name: String,
    gpu: Option<f64>,
    cpu_temperature: Option<f64>,
    gpu_temperature: Option<f64>,
}

pub(crate) struct HardwareMonitor {
    latest: Arc<Mutex<Metrics>>,
    task: tokio::task::JoinHandle<()>,
}

impl HardwareMonitor {
    pub(crate) fn start(frames: FrameRequester) -> Option<Self> {
        if std::env::var("EVO_CODEX_HARDWARE_PANEL").as_deref() == Ok("0") {
            return None;
        }
        let latest = Arc::new(Mutex::new(Metrics::default()));
        let target = latest.clone();
        let task = tokio::spawn(async move {
            let mut sampler = sample::State::default();
            loop {
                let metrics = sample::read(&mut sampler).await;
                if let Ok(mut snapshot) = target.lock() {
                    *snapshot = metrics;
                }
                frames.schedule_frame();
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
        Some(Self { latest, task })
    }

    pub(crate) fn snapshot(&self) -> Metrics {
        self.latest
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }
}

impl Drop for HardwareMonitor {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) fn panel_area(size: Size) -> Option<Rect> {
    (size.width >= 110 && size.height >= 16).then(|| {
        Rect::new(
            size.width - 29,
            /*y*/ 0,
            /*width*/ 29,
            size.height,
        )
    })
}

pub(crate) fn render(metrics: &Metrics, area: Rect, buffer: &mut Buffer) {
    let percent = |value: Option<f64>| value.map_or_else(|| "N/A".into(), |n| format!("{n:.0}%"));
    let temperature =
        |value: Option<f64>| value.map_or_else(|| "N/A".into(), |n| format!("{n:.0} °C"));
    let ram = metrics.ram.map_or_else(
        || "N/A".into(),
        |(used, total)| {
            format!(
                "{:.1} / {:.1} GiB",
                used as f64 / 1_073_741_824.0,
                total as f64 / 1_073_741_824.0
            )
        },
    );
    let lines: Vec<Line> = vec![
        "CPU".bold().into(),
        metrics.cpu_name.clone().into(),
        format!("Usage  {}", percent(metrics.cpu)).into(),
        format!("Temp   {}", temperature(metrics.cpu_temperature)).into(),
        "".into(),
        "RAM".bold().into(),
        ram.into(),
        "".into(),
        "GPU".bold().into(),
        metrics.gpu_name.clone().into(),
        format!("Usage  {}", percent(metrics.gpu)).into(),
        format!("Temp   {}", temperature(metrics.gpu_temperature)).into(),
        "".into(),
        "System-wide measurements".dim().into(),
        "N/A: sensor unavailable".dim().into(),
    ];
    ratatui::widgets::Clear.render(area, buffer);
    Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Hardware ")
                .borders(Borders::LEFT)
                .padding(ratatui::widgets::Padding::horizontal(1)),
        )
        .render(area, buffer);
}

#[cfg(test)]
#[path = "hardware_tests.rs"]
mod tests;
