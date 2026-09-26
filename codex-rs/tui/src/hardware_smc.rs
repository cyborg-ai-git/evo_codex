//! Read-only AppleSMC temperature sampling through IOKit.
//!
//! The SMC user-client ABI uses an 80-byte message (key info at offset 28,
//! command at 42, index at 44, payload at 48). Only read commands are exposed.
//! Sensor families follow the mappings maintained by exelban/Stats:
//! https://github.com/exelban/stats/blob/master/Modules/Sensors/values.swift
use std::ffi::c_char;
use std::ffi::c_void;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(port: u32, matching: *mut c_void) -> u32;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, connection: *mut u32) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IOServiceClose(connection: u32) -> i32;
    fn IOConnectCallStructMethod(
        connection: u32,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> i32;
    static mach_task_self_: u32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Cpu,
    Gpu,
}

fn group(key: [u8; 4]) -> Option<Group> {
    match &key {
        [b'T', b'p' | b'e', _, _] | [b'T', b'f', b'0' | b'4', _] => Some(Group::Cpu),
        [b'T', b'g', _, _] | [b'T', b'f', b'1' | b'2', _] => Some(Group::Gpu),
        b"TC0D" | b"TC0E" | b"TC0F" | b"TCAD" => Some(Group::Cpu),
        b"TG0D" | b"TGDD" => Some(Group::Gpu),
        _ => None,
    }
}

fn temperature(kind: [u8; 4], data: &[u8]) -> Option<f64> {
    let value = match (&kind, data) {
        (b"flt ", [a, b, c, d]) => f64::from(f32::from_le_bytes([*a, *b, *c, *d])),
        (b"sp78", [a, b]) => f64::from(i16::from_be_bytes([*a, *b])) / 256.0,
        _ => return None,
    };
    // Inactive Apple Silicon sensors can return zero or low sentinel values.
    (value.is_finite() && (10.0..=125.0).contains(&value)).then_some(value)
}

#[repr(C, align(4))]
struct Message([u8; 80]);

struct Sensor {
    key: [u8; 4],
    kind: [u8; 4],
    size: usize,
    group: Group,
}

pub(super) struct Smc {
    connection: u32,
    sensors: Vec<Sensor>,
}

impl Smc {
    pub(super) fn open() -> Option<Self> {
        // SAFETY: the class name is NUL terminated; IOKit consumes the matching
        // dictionary. The service and connection are released on all paths.
        let connection = unsafe {
            let matching = IOServiceMatching(c"AppleSMC".as_ptr());
            if matching.is_null() {
                return None;
            }
            let service = IOServiceGetMatchingService(0, matching);
            if service == 0 {
                return None;
            }
            let mut connection = 0;
            let status = IOServiceOpen(service, mach_task_self_, 0, &mut connection);
            IOObjectRelease(service);
            if status != 0 {
                return None;
            }
            connection
        };
        let mut smc = Self {
            connection,
            sensors: Vec::new(),
        };
        let (kind, size) = smc.info(*b"#KEY")?;
        if kind != *b"ui32" || size != 4 {
            return None;
        }
        let count = smc.read(*b"#KEY", size)?;
        let count = u32::from_be_bytes(count.0[48..52].try_into().ok()?);
        // Bound discovery if the driver returns corrupt metadata.
        if count > 16_384 {
            return None;
        }
        for index in 0..count {
            let mut request = Message([0; 80]);
            request.0[42] = 8; // Read key at index.
            request.0[44..48].copy_from_slice(&index.to_ne_bytes());
            let Some(reply) = smc.call(request) else {
                continue;
            };
            let key = u32::from_ne_bytes(reply.0[..4].try_into().ok()?).to_be_bytes();
            let Some(group) = group(key) else { continue };
            let Some((kind, size)) = smc.info(key) else {
                continue;
            };
            if matches!((&kind, size), (b"flt ", 4) | (b"sp78", 2)) {
                smc.sensors.push(Sensor {
                    key,
                    kind,
                    size,
                    group,
                });
            }
        }
        Some(smc)
    }

    fn call(&self, request: Message) -> Option<Message> {
        let mut reply = Message([0; 80]);
        let mut size = std::mem::size_of::<Message>();
        // SAFETY: both aligned buffers remain alive for this synchronous call;
        // sizes match their allocations and output capacity is passed by reference.
        let status = unsafe {
            IOConnectCallStructMethod(
                self.connection,
                2,
                (&raw const request).cast(),
                std::mem::size_of::<Message>(),
                (&raw mut reply).cast(),
                &mut size,
            )
        };
        (status == 0 && size == 80 && reply.0[40] == 0).then_some(reply)
    }

    fn info(&self, key: [u8; 4]) -> Option<([u8; 4], usize)> {
        let mut request = Message([0; 80]);
        request.0[..4].copy_from_slice(&u32::from_be_bytes(key).to_ne_bytes());
        request.0[42] = 9; // Read key metadata.
        let reply = self.call(request)?;
        let size = u32::from_ne_bytes(reply.0[28..32].try_into().ok()?) as usize;
        let kind = u32::from_ne_bytes(reply.0[32..36].try_into().ok()?).to_be_bytes();
        (size <= 32).then_some((kind, size))
    }

    fn read(&self, key: [u8; 4], size: usize) -> Option<Message> {
        let mut request = Message([0; 80]);
        request.0[..4].copy_from_slice(&u32::from_be_bytes(key).to_ne_bytes());
        request.0[28..32].copy_from_slice(&(size as u32).to_ne_bytes());
        request.0[42] = 5; // Read bytes; never write SMC values.
        self.call(request)
    }

    pub(super) fn temperatures(&self) -> (Option<f64>, Option<f64>) {
        let mut cpu: Option<f64> = None;
        let mut gpu: Option<f64> = None;
        for sensor in &self.sensors {
            let Some(reply) = self.read(sensor.key, sensor.size) else {
                continue;
            };
            let Some(value) = temperature(sensor.kind, &reply.0[48..48 + sensor.size]) else {
                continue;
            };
            let hottest = match sensor.group {
                Group::Cpu => &mut cpu,
                Group::Gpu => &mut gpu,
            };
            *hottest = Some(hottest.map_or(value, |previous| previous.max(value)));
        }
        (cpu, gpu)
    }
}

impl Drop for Smc {
    fn drop(&mut self) {
        // SAFETY: this object uniquely owns the connection returned by IOKit.
        unsafe {
            IOServiceClose(self.connection);
        }
    }
}

#[cfg(test)]
#[path = "hardware_smc_tests.rs"]
mod tests;
