use super::*;
use pretty_assertions::assert_eq;

#[test]
fn hardware_panel_renders_real_values_and_missing_sensors() {
    let metrics = Metrics {
        cpu_name: "Apple M5".into(),
        cpu: Some(23.0),
        ram: Some((16 * 1_073_741_824, 32 * 1_073_741_824)),
        gpu_name: "Apple M5".into(),
        gpu: Some(58.0),
        ..Metrics::default()
    };
    let mut buffer = Buffer::empty(Rect::new(0, 0, 29, 18));
    render(&metrics, buffer.area, &mut buffer);
    let text = (0..18)
        .map(|y| (0..29).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(text);
}

#[test]
fn narrow_terminals_keep_the_chat_width() {
    assert_eq!(panel_area(Size::new(109, 30)), None);
    assert_eq!(
        panel_area(Size::new(140, 30)),
        Some(Rect::new(111, 0, 29, 30))
    );
}

#[test]
fn parsers_use_the_latest_sample_and_preserve_unavailable_values() {
    assert_eq!(
        sample::mac_cpu(
            "CPU usage: 10% user, 20% sys, 70% idle\nCPU usage: 5% user, 5% sys, 90% idle"
        ),
        Some(10.0)
    );
    assert_eq!(
        sample::mac_gpu("\"Device Utilization %\"=0,\"Other\"=4"),
        Some(0.0)
    );
    assert_eq!(sample::mac_gpu("no counter"), None);
    assert_eq!(sample::percentage("NaN"), None);
}

#[test]
fn hardware_panel_renders_cpu_and_gpu_temperatures() {
    let metrics = Metrics {
        cpu_name: "Apple M5".into(),
        cpu: Some(23.0),
        ram: Some((16 * 1_073_741_824, 32 * 1_073_741_824)),
        gpu_name: "Apple M5".into(),
        gpu: Some(58.0),
        cpu_temperature: Some(43.4),
        gpu_temperature: Some(42.7),
    };
    let mut buffer = Buffer::empty(Rect::new(0, 0, 29, 18));
    render(&metrics, buffer.area, &mut buffer);
    let text = (0..18)
        .map(|y| (0..29).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    insta::assert_snapshot!(text);
}
