// Mojo LA: talks SUMP to the logic_analyzer bitstream over the Mojo's USB serial port.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::time::{Duration, Instant};

const SUMP_CLOCK: u32 = 100_000_000; // SUMP dividers are relative to 100 MHz

fn long(cmd: u8, arg: u32) -> [u8; 5] {
    let a = arg.to_le_bytes();
    [cmd, a[0], a[1], a[2], a[3]]
}

/// SUMP setup for one capture. `samples` is a multiple of 4; `post` of them come after the trigger.
fn setup(rate: u32, samples: u32, post: u32, mask: u8, value: u8) -> Vec<u8> {
    let post = post.clamp(4, samples);
    let size = (samples / 4 - 1) | ((post / 4 - 1) << 16);
    [
        long(0x80, SUMP_CLOCK / rate - 1),
        long(0x81, size),
        long(0xC0, mask as u32),
        long(0xC1, value as u32),
    ]
    .concat()
}

#[tauri::command]
fn list_ports() -> Vec<String> {
    let mut ports: Vec<String> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .filter(|n| {
            !n.starts_with("/dev/tty.") && !n.contains("Bluetooth") && !n.contains("debug-console")
        })
        .collect();
    ports.sort_by_key(|n| !n.contains("usbmodem")); // the Mojo first
    ports
}

fn run(
    port: &str,
    rate: u32,
    samples: u32,
    post: u32,
    mask: u8,
    value: u8,
    wait_s: u64,
) -> Result<Vec<u8>, String> {
    let io = |e: std::io::Error| format!("{port}: {e}");
    let mut p = serialport::new(port, 115_200)
        .timeout(Duration::from_millis(300))
        .open()
        .map_err(|e| format!("Can't open {port}: {e}"))?;

    p.write_all(&[0; 5]).map_err(io)?; // reset, also aborts a capture left armed
    std::thread::sleep(Duration::from_millis(50));
    let _ = p.clear(serialport::ClearBuffer::Input);

    p.write_all(&[0x02]).map_err(io)?;
    let mut id = [0u8; 4];
    if p.read_exact(&mut id).is_err() || &id != b"1ALS" {
        return Err(format!(
            "No logic analyzer answered on {port}. Is the logic_analyzer bitstream flashed?"
        ));
    }

    p.write_all(&setup(rate, samples, post, mask, value))
        .map_err(io)?;
    p.write_all(&[0x01]).map_err(io)?;

    let mut buf = vec![0u8; samples as usize];
    let mut got = 0;
    let deadline = Instant::now() + Duration::from_secs(wait_s);
    while got < buf.len() {
        match p.read(&mut buf[got..]) {
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if Instant::now() > deadline {
                    let _ = p.write_all(&[0; 5]);
                    return Err(if got == 0 {
                        format!("Trigger didn't fire within {wait_s} s.")
                    } else {
                        format!("Capture stopped after {got} of {samples} samples.")
                    });
                }
            }
            Err(e) => return Err(io(e)),
        }
    }
    buf.reverse(); // SUMP sends newest first
    Ok(buf)
}

#[tauri::command]
async fn capture(
    port: String,
    rate: u32,
    samples: u32,
    post: u32,
    mask: u8,
    value: u8,
    wait_s: u64,
) -> Result<Vec<u8>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run(&port, rate, samples, post, mask, value, wait_s)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_ports, capture])
        .run(tauri::generate_context!())
        .expect("error while running Mojo LA");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_bytes() {
        // 1 MHz, 16K samples, 4K after the trigger, trigger on D0 high
        let b = setup(1_000_000, 16384, 4096, 0x01, 0x01);
        assert_eq!(&b[0..5], &[0x80, 99, 0, 0, 0]);
        assert_eq!(&b[5..10], &[0x81, 0xFF, 0x0F, 0xFF, 0x03]);
        assert_eq!(&b[10..20], &[0xC0, 1, 0, 0, 0, 0xC1, 1, 0, 0, 0]);
        // post is clamped to the 4..=samples range the hardware counts in
        assert_eq!(
            &setup(50_000_000, 1024, 0, 0, 0)[5..10],
            &[0x81, 0xFF, 0x00, 0x00, 0x00]
        );
    }

    /// Needs a Mojo with the logic_analyzer bitstream: cargo test -- --ignored
    #[test]
    #[ignore]
    fn board_capture() {
        let port = list_ports().into_iter().next().expect("no serial port");
        let d = run(&port, 1_000_000, 16384, 16384, 0, 0, 5).unwrap();
        assert_eq!(d.len(), 16384);
    }
}
