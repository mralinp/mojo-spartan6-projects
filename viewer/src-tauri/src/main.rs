// Mojo LA: talks SUMP to the logic_analyzer bitstream over the Mojo's USB serial port.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SUMP_CLOCK: u32 = 100_000_000; // SUMP dividers are relative to 100 MHz
/// Rates the hardware hits exactly: they divide the analyzer's 200 MHz clock, and up to 100 MHz also
/// SUMP's 100 MHz reference. Keep in sync with ui/index.html.
const RATES: [u32; 14] = [
    200_000_000,
    100_000_000,
    50_000_000,
    25_000_000,
    10_000_000,
    5_000_000,
    2_000_000,
    1_000_000,
    500_000,
    200_000,
    100_000,
    50_000,
    10_000,
    1_000,
];

type Port = Box<dyn serialport::SerialPort>;

fn long(cmd: u8, arg: u32) -> [u8; 5] {
    let a = arg.to_le_bytes();
    [cmd, a[0], a[1], a[2], a[3]]
}

/// One capture's settings, as the UI sends them. `samples` is a multiple of 4; `post` of them come after the trigger.
#[derive(serde::Deserialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
struct Settings {
    rate: u32,
    samples: u32,
    post: u32,
    mask: u16,
    value: u16,
    wait_ms: u64,
    /// 8 or 16. Asking for 8 sends one byte per sample: half the transfer time.
    #[serde(default = "eight")]
    channels: u8,
}
fn eight() -> u8 {
    8
}

/// SUMP setup commands for one capture.
fn setup(s: &Settings) -> Vec<u8> {
    let post = s.post.clamp(4, s.samples);
    let size = (s.samples / 4 - 1) | ((post / 4 - 1) << 16);
    // Above 100 MHz the divider counts the analyzer's 200 MHz clock instead (SUMP's demux flag).
    let demux = s.rate > SUMP_CLOCK;
    let div = if demux {
        2 * SUMP_CLOCK / s.rate - 1
    } else {
        SUMP_CLOCK / s.rate - 1
    };
    // Flags: bit 0 demux; bits 2-5 turn channel groups 0-3 off. Groups 2-3 don't exist here.
    let groups_off: u32 = if s.channels > 8 { 0x30 } else { 0x38 }; // 16: groups 2-3 off; 8: groups 1-3 off
    [
        long(0x82, demux as u32 | groups_off),
        long(0x80, div),
        long(0x81, size),
        long(0xC0, s.mask as u32),
        long(0xC1, s.value as u32),
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

/// Opens `name` and checks that a logic analyzer answers.
fn connect(name: &str) -> Result<Port, String> {
    let io = |e: std::io::Error| format!("{name}: {e}");
    let mut p = serialport::new(name, 115_200)
        .timeout(Duration::from_millis(300))
        .open()
        .map_err(|e| format!("Can't open {name}: {e}"))?;
    p.write_all(&[0; 5]).map_err(io)?; // reset, also aborts a capture left armed
    std::thread::sleep(Duration::from_millis(50));
    let _ = p.clear(serialport::ClearBuffer::Input);
    p.write_all(&[0x02]).map_err(io)?;
    let mut id = [0u8; 4];
    if p.read_exact(&mut id).is_err() || &id != b"1ALS" {
        return Err(format!(
            "No logic analyzer answered on {name}. Is the logic_analyzer bitstream flashed?"
        ));
    }
    Ok(p)
}

/// One capture, oldest sample first. `None` when the trigger didn't fire within `wait_ms`.
fn capture_on(p: &mut Port, s: &Settings) -> Result<Option<Vec<u16>>, String> {
    let io = |e: std::io::Error| e.to_string();
    let _ = p.clear(serialport::ClearBuffer::Input);
    p.write_all(&setup(s)).map_err(io)?;
    p.write_all(&[0x01]).map_err(io)?;

    let width = if s.channels > 8 { 2 } else { 1 }; // bytes per sample, low group first
    let mut buf = vec![0u8; s.samples as usize * width];
    let mut got = 0;
    let deadline = Instant::now() + Duration::from_millis(s.wait_ms);
    while got < buf.len() {
        match p.read(&mut buf[got..]) {
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                if Instant::now() > deadline {
                    if got > 0 {
                        return Err(format!(
                            "Capture stopped after {} of {} samples.",
                            got / width,
                            s.samples
                        ));
                    }
                    p.write_all(&[0; 5]).map_err(io)?; // disarm; drop anything that raced in
                    std::thread::sleep(Duration::from_millis(20));
                    let _ = p.clear(serialport::ClearBuffer::Input);
                    return Ok(None);
                }
            }
            Err(e) => return Err(io(e)),
        }
    }
    let mut out: Vec<u16> = buf
        .chunks(width)
        .map(|c| c[0] as u16 | c.get(1).map_or(0, |&h| (h as u16) << 8))
        .collect();
    out.reverse(); // SUMP sends newest first
    Ok(Some(out))
}

/// The open connection, kept between captures. Reopening the port toggles DTR,
/// and five DTR drops within 250 ms send the Mojo's AVR into its loader.
#[derive(Default, Clone)]
struct Link(Arc<Mutex<Option<(String, Port)>>>);

impl Link {
    /// Runs `f` on an identified connection to `name`, (re)connecting when needed.
    fn with<T>(
        &self,
        name: &str,
        f: impl FnOnce(&mut Port) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut g = self.0.lock().unwrap();
        if g.as_ref().is_none_or(|(n, _)| n != name) {
            *g = None;
            *g = Some((name.to_string(), connect(name)?));
        }
        let r = f(&mut g.as_mut().unwrap().1);
        if r.is_err() {
            *g = None; // reconnect and re-identify next time
        }
        r
    }
}

/// Per-channel summary of a capture.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Activity {
    edges: usize,
    /// The level the line spends most time at.
    idle_high: bool,
    /// Shortest complete pulse in samples (runs cut off by the capture's ends don't count).
    min_run: Option<usize>,
}

fn activity(d: &[u16], channels: usize) -> Vec<Activity> {
    let mut out = vec![Activity::default(); channels];
    for (ch, a) in out.iter_mut().enumerate() {
        let bit = |i: usize| (d[i] >> ch) & 1;
        let highs = (0..d.len()).filter(|&i| bit(i) == 1).count();
        a.idle_high = highs * 2 > d.len();
        let mut last_edge = None;
        for i in 1..d.len() {
            if bit(i) != bit(i - 1) {
                a.edges += 1;
                if let Some(l) = last_edge {
                    let run = i - l;
                    a.min_run = Some(a.min_run.map_or(run, |m: usize| m.min(run)));
                }
                last_edge = Some(i);
            }
        }
    }
    out
}

/// Slowest exact rate that still gives 10 samples per shortest pulse.
fn pick_rate(pulse_s: f64) -> u32 {
    let need = 10.0 / pulse_s;
    RATES
        .iter()
        .rev()
        .copied()
        .find(|&r| r as f64 >= need)
        .unwrap_or(RATES[0])
}

/// What Auto found and chose.
#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Setup {
    rate: u32,
    channel: u8,
    idle_high: bool,
    pulse_ns: f64,
}

fn autoset_on(p: &mut Port, channels: u8) -> Result<Setup, String> {
    // 1. Which lines move, and where do they rest? 0.4 s at 10 kHz.
    // ponytail: a lone pulse shorter than 100 µs can slip between these samples; a faster second look would catch it
    let look = Settings {
        rate: 10_000,
        samples: 4096,
        post: 4096,
        mask: 0,
        value: 0,
        wait_ms: 2000,
        channels,
    };
    let d = capture_on(p, &look)?.ok_or("The analyzer didn't answer.")?;
    let a = activity(&d, channels as usize);
    let ch = (0..a.len()).max_by_key(|&c| a[c].edges).unwrap();
    if a[ch].edges == 0 {
        return Err(
            "No activity on any probe for 0.4 s. Check the wiring and the shared ground.".into(),
        );
    }
    let idle_high = a[ch].idle_high;
    let mask = 1u16 << ch;

    // 2. Shortest pulse: trigger on that line leaving idle, fastest rate first, until a whole pulse fits.
    for rate in [200_000_000, 20_000_000, 2_000_000, 200_000, 20_000] {
        let s = Settings {
            rate,
            samples: 4096,
            post: 3688,
            mask,
            value: if idle_high { 0 } else { mask },
            wait_ms: 3000,
            channels,
        };
        let d =
            capture_on(p, &s)?.ok_or(format!("D{ch} went quiet before it could be measured."))?;
        if let Some(run) = activity(&d, channels as usize)
            .iter()
            .filter_map(|a| a.min_run)
            .min()
        {
            let pulse = run as f64 / rate as f64;
            return Ok(Setup {
                rate: pick_rate(pulse),
                channel: ch as u8,
                idle_high,
                pulse_ns: pulse * 1e9,
            });
        }
    }
    Err(format!("D{ch} toggles too rarely to measure a pulse."))
}

#[tauri::command]
async fn capture(
    link: tauri::State<'_, Link>,
    port: String,
    s: Settings,
) -> Result<Option<Vec<u16>>, String> {
    let link = link.inner().clone();
    tauri::async_runtime::spawn_blocking(move || link.with(&port, |p| capture_on(p, &s)))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn autoset(
    link: tauri::State<'_, Link>,
    port: String,
    channels: Option<u8>,
) -> Result<Setup, String> {
    let link = link.inner().clone();
    let channels = channels.unwrap_or(8);
    tauri::async_runtime::spawn_blocking(move || link.with(&port, |p| autoset_on(p, channels)))
        .await
        .map_err(|e| e.to_string())?
}

/// Files the UI may write or read: only our own formats, never an arbitrary path's contents.
fn checked_path(path: &str, allowed: &[&str]) -> Result<std::path::PathBuf, String> {
    let p = std::path::PathBuf::from(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if allowed.contains(&ext.as_str()) {
        Ok(p)
    } else {
        Err(format!(
            "Mojo LA only handles .{} files.",
            allowed.join(", .")
        ))
    }
}

/// Saves a capture or an export to a path the user picked in the save dialog.
#[tauri::command]
fn write_text(path: String, contents: String) -> Result<(), String> {
    let p = checked_path(&path, &["mojola", "vcd", "csv"])?;
    std::fs::write(&p, contents).map_err(|e| format!("Couldn't save {}: {e}", p.display()))
}

/// Reads a capture the user picked in the open dialog.
#[tauri::command]
fn read_text(path: String) -> Result<String, String> {
    let p = checked_path(&path, &["mojola"])?;
    std::fs::read_to_string(&p).map_err(|e| format!("Couldn't open {}: {e}", p.display()))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Link::default())
        .invoke_handler(tauri::generate_handler![
            list_ports, capture, autoset, write_text, read_text
        ])
        .run(tauri::generate_context!())
        .expect("error while running Mojo LA");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(rate: u32, samples: u32, post: u32, mask: u16, value: u16) -> Settings {
        Settings {
            rate,
            samples,
            post,
            mask,
            value,
            wait_ms: 5000,
            channels: 8,
        }
    }

    #[test]
    fn setup_bytes() {
        // 1 MHz, 16K samples, 4K after the trigger, trigger on D0 high
        let b = setup(&settings(1_000_000, 16384, 4096, 0x01, 0x01));
        assert_eq!(&b[0..5], &[0x82, 0x38, 0, 0, 0]); // demux off, groups 1-3 off: 8 channels
        assert_eq!(&b[5..10], &[0x80, 99, 0, 0, 0]);
        assert_eq!(&b[10..15], &[0x81, 0xFF, 0x0F, 0xFF, 0x03]);
        assert_eq!(&b[15..25], &[0xC0, 1, 0, 0, 0, 0xC1, 1, 0, 0, 0]);
        // 200 MHz: demux on, divider 0 counts the 200 MHz clock; 100 MHz: plain SUMP, divider 0
        let fast = setup(&settings(200_000_000, 1024, 1024, 0, 0));
        assert_eq!(&fast[0..10], &[0x82, 0x39, 0, 0, 0, 0x80, 0, 0, 0, 0]);
        let hundred = setup(&settings(100_000_000, 1024, 1024, 0, 0));
        assert_eq!(&hundred[0..10], &[0x82, 0x38, 0, 0, 0, 0x80, 0, 0, 0, 0]);
        // 16 channels: only groups 2-3 off, and a 16-bit trigger mask
        let wide = setup(&Settings {
            channels: 16,
            ..settings(1_000_000, 1024, 1024, 0x8001, 0x8000)
        });
        assert_eq!(&wide[0..5], &[0x82, 0x30, 0, 0, 0]);
        assert_eq!(
            &wide[15..25],
            &[0xC0, 0x01, 0x80, 0, 0, 0xC1, 0x00, 0x80, 0, 0]
        );
        // post is clamped to the 4..=samples range the hardware counts in
        assert_eq!(
            &setup(&settings(50_000_000, 1024, 0, 0, 0))[10..15],
            &[0x81, 0xFF, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn file_paths() {
        assert!(checked_path("/tmp/a.mojola", &["mojola"]).is_ok());
        assert!(checked_path("/tmp/A.VCD", &["vcd"]).is_ok());
        assert!(checked_path("/etc/passwd", &["mojola", "vcd", "csv"]).is_err());
        assert!(checked_path("/tmp/a.csv", &["mojola"]).is_err());
        let dir = std::env::temp_dir().join("mojo-la-test.mojola");
        write_text(dir.to_string_lossy().into(), "{}".into()).unwrap();
        assert_eq!(read_text(dir.to_string_lossy().into()).unwrap(), "{}");
        let _ = std::fs::remove_file(dir);
    }

    #[test]
    fn activity_summary() {
        // D0: idle high with a 3-sample low pulse and a 5-sample low pulse; D1: flat low
        let d: Vec<u16> = "1111000111110000011111"
            .bytes()
            .map(|c| (c - b'0') as u16 | 0x8000)
            .collect();
        let a = activity(&d, 16);
        assert_eq!(
            a[0],
            Activity {
                edges: 4,
                idle_high: true,
                min_run: Some(3)
            }
        );
        assert_eq!(
            a[1],
            Activity {
                edges: 0,
                idle_high: false,
                min_run: None
            }
        );
        // a run cut off by the capture's end isn't a pulse
        assert_eq!(activity(&[1, 1, 0, 0], 8)[0].min_run, None);
        // D15 held high the whole time: idle high, no edges
        assert_eq!(
            a[15],
            Activity {
                edges: 0,
                idle_high: true,
                min_run: None
            }
        );
    }

    #[test]
    fn rate_choice() {
        assert_eq!(pick_rate(1.0 / 115_200.0), 2_000_000); // UART bit 8.7 µs -> 10 samples needs 1.15 MHz
        assert_eq!(pick_rate(1e-6), 10_000_000);
        assert_eq!(pick_rate(100e-9), 100_000_000);
        assert_eq!(pick_rate(50e-9), 200_000_000);
        assert_eq!(pick_rate(10e-9), 200_000_000); // faster than we can sample: best we have
        assert_eq!(pick_rate(1.0), 1_000);
    }

    // Need a Mojo with the logic_analyzer bitstream: cargo test -- --ignored
    fn board() -> Link {
        Link::default()
    }
    fn port() -> String {
        list_ports().into_iter().next().expect("no serial port")
    }

    #[test]
    #[ignore]
    fn board_capture() {
        let d = board().with(&port(), |p| {
            capture_on(p, &settings(1_000_000, 16384, 16384, 0, 0))
        });
        assert_eq!(d.unwrap().unwrap().len(), 16384);
    }

    /// 16 channels: two bytes per sample come back as one 16-bit sample each. With the Pro Micro rig,
    /// D1 carries the UART and D8-D15 (nothing wired, pulled down) stay low.
    #[test]
    #[ignore]
    fn board_capture_16() {
        let s = Settings {
            channels: 16,
            ..settings(1_000_000, 16384, 14748, 2, 0)
        };
        let d = board()
            .with(&port(), |p| capture_on(p, &s))
            .unwrap()
            .expect("no UART traffic on D1");
        assert_eq!(d.len(), 16384);
        let a = activity(&d, 16);
        assert!(a[1].edges > 0, "D1 should toggle");
        assert!(
            a[8..].iter().all(|c| c.edges == 0 && !c.idle_high),
            "D8-D15 should sit low"
        );
    }

    /// 30 back-to-back captures on one connection: the analyzer must still answer afterwards.
    #[test]
    #[ignore]
    fn board_live() {
        let link = board();
        let t = Instant::now();
        for _ in 0..30 {
            let d = link.with(&port(), |p| {
                capture_on(p, &settings(1_000_000, 1024, 1024, 0, 0))
            });
            assert_eq!(d.unwrap().unwrap().len(), 1024);
        }
        println!("30 captures of 1K in {:.2} s", t.elapsed().as_secs_f64());
        drop(link); // the port is exclusive: close it before checking with a fresh connection
        assert!(connect(&port()).is_ok());
    }

    /// With the Pro Micro UART rig wired up: Auto should find the 115200-baud line.
    #[test]
    #[ignore]
    fn board_autoset() {
        let s = board().with(&port(), |p| autoset_on(p, 8)).unwrap();
        println!("{s:?}");
        assert_eq!(s.rate, 2_000_000);
        assert!(s.idle_high);
        assert!(
            (8_000.0..9_500.0).contains(&s.pulse_ns),
            "one bit at 115200 is 8.68 µs"
        );
    }

    /// With the Pro Micro rig on D1. A 16 MHz AVR can't make 115200 exactly: it runs at
    /// 16 MHz / (8 * 17) = 117647 baud, an 8.50 µs bit. The measured shortest pulse proves the
    /// real sample rate: 1700 samples at 200 MHz, 850 at 100 MHz.
    #[test]
    #[ignore]
    fn board_rates() {
        let link = board();
        for (rate, want) in [(200_000_000, 1700.0), (100_000_000, 850.0)] {
            let s = Settings {
                channels: 8,
                rate,
                samples: 16384,
                post: 14748,
                mask: 2,
                value: 0,
                wait_ms: 3000,
            };
            let d = link
                .with(&port(), |p| capture_on(p, &s))
                .unwrap()
                .expect("no UART traffic on D1");
            let run = activity(&d, 8)[1].min_run.expect("no whole bit on D1") as f64;
            println!(
                "{} MHz: shortest pulse on D1 = {run} samples (want ~{want})",
                rate / 1_000_000
            );
            assert!((run - want).abs() / want < 0.01);
        }
    }
}
