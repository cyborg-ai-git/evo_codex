use super::*;
use pretty_assertions::assert_eq;

#[test]
fn decodes_apple_silicon_float_and_intel_fixed_point() {
    assert_eq!(
        temperature(*b"flt ", &43.4375_f32.to_le_bytes()),
        Some(43.4375)
    );
    assert_eq!(temperature(*b"sp78", &[0x35, 0x80]), Some(53.5));
}

#[test]
fn invalid_or_inactive_readings_remain_unavailable() {
    for value in [
        f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        -20.0,
        0.0,
        2.3,
        200.0,
    ] {
        assert_eq!(temperature(*b"flt ", &value.to_le_bytes()), None);
    }
    assert_eq!(temperature(*b"sp78", &[0xff, 0x00]), None);
    assert_eq!(temperature(*b"flt ", &[1, 2]), None);
    assert_eq!(temperature(*b"sp78", &[1, 2, 3, 4]), None);
    assert_eq!(temperature(*b"ui32", &[0, 0, 0, 50]), None);
}

#[test]
fn classifies_die_sensors_without_using_battery_or_memory_temperatures() {
    assert_eq!(
        [*b"Tp00", *b"Te05", *b"Tf04", *b"Tf49", *b"TCAD"].map(group),
        [Some(Group::Cpu); 5]
    );
    assert_eq!(
        [*b"Tg0U", *b"Tg1g", *b"Tf14", *b"Tf29", *b"TG0D"].map(group),
        [Some(Group::Gpu); 5]
    );
    assert_eq!(
        [*b"TB1T", *b"Tm0p", *b"TaLP", *b"PSTR"].map(group),
        [None; 4]
    );
}
