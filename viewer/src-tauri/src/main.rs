// Mojo LA: talks SUMP to the logic_analyzer bitstream over the Mojo's USB serial port.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// Long capture (8 channels): the analyzer stores {run length - 1, value} entries, two bytes each,
    /// only when the inputs change. `samples` then counts entries.
    #[serde(default)]
    rle: bool,
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
    let two_bytes = s.channels > 8 || s.rle;
    let groups_off: u32 = if two_bytes { 0x30 } else { 0x38 }; // groups 2-3 off; for one byte, group 1 too
    let rle = if s.rle { 0x4000 } else { 0 }; // flags bit 14: run-length entries
    [
        long(0x82, demux as u32 | groups_off | rle),
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

    let width = if s.channels > 8 || s.rle { 2 } else { 1 }; // bytes per sample (or RLE entry), low first
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
        rle: false,
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
            rle: false,
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

/// One live-log receiver: a UART on channel `ch`.
#[derive(serde::Deserialize, Clone, Copy, Debug)]
struct LiveRx {
    ch: u8,
    baud: u32,
}

/// What the live log streams: a byte from receiver `rx`, or `dropped` bytes the analyzer couldn't keep.
#[derive(serde::Serialize, Clone, Copy, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
struct LiveRec {
    t_us: u64,
    rx: u8,
    byte: u8,
    err: bool,
    dropped: u8,
}

/// Turns the analyzer's 2-byte live records back into timestamped bytes (see live_log in mojo_top.v).
#[derive(Default)]
struct LiveParser {
    t: u64, // 10 us steps since the start
    hi: Option<u8>,
}

impl LiveParser {
    fn feed(&mut self, bytes: &[u8], out: &mut Vec<LiveRec>) {
        for &b in bytes {
            let Some(h) = self.hi.take() else {
                self.hi = Some(b);
                continue;
            };
            if h == 0x7F {
                out.push(LiveRec {
                    t_us: self.t * 10,
                    dropped: b,
                    ..Default::default()
                });
            } else if h & 0x80 == 0 {
                self.t += ((h as u64) << 8) | b as u64;
            } else {
                self.t += (h & 0x1F) as u64;
                out.push(LiveRec {
                    t_us: self.t * 10,
                    rx: (h >> 6) & 1,
                    byte: b,
                    err: h & 0x20 != 0,
                    dropped: 0,
                });
            }
        }
    }
}

/// Streams the live log until `stop`, handing records to `sink` about every 50 ms.
fn live_on(
    p: &mut Port,
    rxs: &[LiveRx],
    stop: &AtomicBool,
    mut sink: impl FnMut(Vec<LiveRec>),
) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let mut cmd = vec![];
    for (i, cmd_id) in [0x90u8, 0x91].into_iter().enumerate() {
        let arg = rxs.get(i).map_or(0, |r| {
            let cpb = (200_000_000 + r.baud / 2) / r.baud.max(1);
            (cpb.min(0xF_FFFF) << 12) | 0x10 | (r.ch as u32 & 15)
        });
        cmd.extend(long(cmd_id, arg));
    }
    cmd.push(0x03);
    let _ = p.clear(serialport::ClearBuffer::Input);
    p.write_all(&cmd).map_err(io)?;

    let mut parser = LiveParser::default();
    let (mut recs, mut buf, mut sent) = (vec![], [0u8; 4096], Instant::now());
    let r = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        match p.read(&mut buf) {
            Ok(n) => parser.feed(&buf[..n], &mut recs),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => break Err(io(e)),
        }
        if !recs.is_empty() && sent.elapsed() > Duration::from_millis(50) {
            sink(std::mem::take(&mut recs));
            sent = Instant::now();
        }
    };
    if !recs.is_empty() {
        sink(recs);
    }
    p.write_all(&[0; 5]).map_err(io)?; // leave live mode
    std::thread::sleep(Duration::from_millis(20));
    let _ = p.clear(serialport::ClearBuffer::Input);
    r
}

/// Set while a live log runs; clearing it stops the stream.
#[derive(Default)]
struct Live(Mutex<Option<Arc<AtomicBool>>>);

/// Starts the live log: records arrive as "live" events, the end (or an error) as "live-end".
#[tauri::command]
fn live_start(
    app: tauri::AppHandle,
    link: tauri::State<'_, Link>,
    live: tauri::State<'_, Live>,
    port: String,
    rxs: Vec<LiveRx>,
) {
    use tauri::Emitter;
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(old) = live.0.lock().unwrap().replace(stop.clone()) {
        old.store(true, Ordering::Relaxed);
    }
    let link = link.inner().clone();
    std::thread::spawn(move || {
        let r = link.with(&port, |p| {
            live_on(p, &rxs, &stop, |recs| {
                let _ = app.emit("live", recs);
            })
        });
        let _ = app.emit("live-end", r.err());
    });
}

#[tauri::command]
fn live_stop(live: tauri::State<'_, Live>) {
    if let Some(stop) = live.0.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }
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
    let p = checked_path(&path, &["mojola", "vcd", "csv", "txt"])?;
    std::fs::write(&p, contents).map_err(|e| format!("Couldn't save {}: {e}", p.display()))
}

/// Reads a capture the user picked in the open dialog.
#[tauri::command]
fn read_text(path: String) -> Result<String, String> {
    let p = checked_path(&path, &["mojola"])?;
    std::fs::read_to_string(&p).map_err(|e| format!("Couldn't open {}: {e}", p.display()))
}

// ---- sigrok decoders: every protocol libsigrokdecode knows, through an installed sigrok-cli ----
// sigrok is GPL-3, so it stays a separate program: we run it, never bundle or link it.

/// sigrok-cli on PATH, or where its installers put it (apps started from the Dock don't get the shell's PATH).
fn sigrok_cli() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        "sigrok-cli.exe"
    } else {
        "sigrok-cli"
    };
    let fixed = [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        r"C:\Program Files\sigrok\sigrok-cli",
        r"C:\Program Files (x86)\sigrok\sigrok-cli",
    ];
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(fixed.iter().map(Into::into))
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
struct PdChannel {
    id: String,
    name: String,
    desc: String,
}

#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
struct PdOption {
    id: String,
    desc: String,
    default: String,
    values: Vec<String>,
}

/// One decoder, from `sigrok-cli -P <id> --show`.
#[derive(serde::Serialize, Clone, Debug, Default, PartialEq)]
struct Decoder {
    id: String,
    name: String,
    longname: String,
    desc: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
    required: Vec<PdChannel>,
    optional: Vec<PdChannel>,
    options: Vec<PdOption>,
    /// Annotation row names, as the decoder output labels them (class ids for decoders without rows).
    rows: Vec<String>,
}

fn parse_show(text: &str) -> Decoder {
    let mut d = Decoder::default();
    let mut classes = vec![];
    let mut section = "";
    for line in text.lines() {
        if let Some(item) = line.strip_prefix("- ") {
            // "id (Name): desc" for channels and rows, "id: desc (values, default x)" for options, "id" for in/outputs
            let (head, rest) = item.split_once(": ").unwrap_or((item, ""));
            let (id, name) = match head.split_once(" (") {
                Some((i, n)) => (i, n.trim_end_matches(')')),
                None => (head, head),
            };
            let ch = || PdChannel {
                id: id.into(),
                name: name.into(),
                desc: rest.into(),
            };
            match section {
                "Possible decoder input IDs" => d.inputs.push(id.into()),
                "Possible decoder output IDs" => d.outputs.push(id.into()),
                "Required channels" => d.required.push(ch()),
                "Optional channels" => d.optional.push(ch()),
                "Annotation rows" => d.rows.push(name.into()),
                "Annotation classes" => classes.push(id.to_string()),
                "Options" => {
                    let mut o = PdOption {
                        id: id.into(),
                        desc: rest.into(),
                        ..Default::default()
                    };
                    if let Some(at) = rest
                        .rfind(" (")
                        .filter(|&at| rest[at..].contains("default "))
                    {
                        o.desc = rest[..at].into();
                        for v in rest[at + 2..].trim_end_matches(')').split(", ") {
                            let unq = |v: &str| v.trim_matches('\'').to_string();
                            match v.strip_prefix("default ") {
                                Some(def) => o.default = unq(def),
                                None => o.values.push(unq(v)),
                            }
                        }
                    }
                    d.options.push(o);
                }
                _ => {}
            }
        } else if let Some((key, val)) = line.split_once(':') {
            section = key;
            let val = val.trim().to_string();
            match key {
                "ID" => d.id = val,
                "Name" => d.name = val,
                "Long name" => d.longname = val,
                "Description" => d.desc = val,
                _ => {}
            }
        }
    }
    if d.rows.is_empty() {
        d.rows = classes; // no rows: sigrok labels each annotation with its class
    }
    d
}

static CATALOG: Mutex<Option<Vec<Decoder>>> = Mutex::new(None);

/// Every installed decoder, sorted by id. Read once per run (about a second), then cached.
#[tauri::command]
async fn sigrok_decoders() -> Result<Vec<Decoder>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        if let Some(c) = CATALOG.lock().unwrap().clone() {
            return Ok(c);
        }
        let cli = sigrok_cli().ok_or("sigrok-cli isn't installed.")?;
        let run = |args: &[&str]| {
            std::process::Command::new(&cli)
                .args(args)
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .map_err(|e| format!("Couldn't run sigrok-cli: {e}"))
        };
        let list = run(&["-L"])?;
        let ids: Vec<String> = list
            .lines()
            .skip_while(|l| !l.starts_with("Supported protocol decoders"))
            .skip(1)
            .take_while(|l| l.starts_with("  "))
            .filter_map(|l| l.split_whitespace().next().map(String::from))
            .collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let found = Mutex::new(vec![]);
        std::thread::scope(|sc| {
            for _ in 0..8 {
                sc.spawn(|| {
                    while let Some(id) = ids.get(next.fetch_add(1, Ordering::Relaxed)) {
                        if let Ok(t) = run(&["-P", id, "--show"]) {
                            let d = parse_show(&t);
                            if d.id == *id {
                                found.lock().unwrap().push(d);
                            }
                        }
                    }
                });
            }
        });
        let mut c = found.into_inner().unwrap();
        c.sort_by(|a, b| a.id.cmp(&b.id));
        if c.is_empty() {
            return Err("sigrok-cli lists no protocol decoders.".into());
        }
        *CATALOG.lock().unwrap() = Some(c.clone());
        Ok(c)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One decoder in a stack: the first gets channels, the rest read the one before.
#[derive(serde::Deserialize, Clone, Debug)]
struct PdSpec {
    id: String,
    #[serde(default)]
    channels: std::collections::BTreeMap<String, u8>,
    #[serde(default)]
    options: std::collections::BTreeMap<String, String>,
}

/// sigrok-cli's -P argument: `uart:rx=1:baudrate=9600,midi`. Its separators can't appear in values.
fn pd_arg(stack: &[PdSpec]) -> Result<String, String> {
    let ok = |s: &str| !s.is_empty() && !s.contains([':', ',', '=']) && !s.starts_with('-');
    let mut parts = vec![];
    for pd in stack {
        if !ok(&pd.id) {
            return Err(format!("Bad decoder name {:?}", pd.id));
        }
        let mut p = pd.id.clone();
        for (k, ch) in &pd.channels {
            p += &format!(":{k}={ch}");
        }
        for (k, v) in &pd.options {
            if !ok(k) || !ok(v) {
                return Err(format!("{}: option {k} can't be {v:?}", pd.id));
            }
            p += &format!(":{k}={v}");
        }
        parts.push(p);
    }
    if parts.is_empty() {
        return Err("No decoder given.".into());
    }
    Ok(parts.join(","))
}

/// A decoder annotation in samples: `pd` is the decoder ("uart-1"), `row` the annotation row's name.
#[derive(serde::Serialize, Clone, Debug, PartialEq)]
struct Ann {
    pd: String,
    row: String,
    start: u64,
    end: u64,
    text: String,
}

/// sigrok-cli's --protocol-decoder-jsontrace output (begin/end events in microseconds) as annotations.
fn parse_trace(json: &str, rate: f64) -> Result<Vec<Ann>, String> {
    if json.trim().is_empty() {
        return Ok(vec![]);
    }
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("sigrok-cli output: {e}"))?;
    let str_of = |e: &serde_json::Value, k: &str| e[k].as_str().unwrap_or("").to_string();
    let at = |e: &serde_json::Value| {
        (e["ts"].as_f64().unwrap_or(0.0) * rate / 1e6)
            .round()
            .max(0.0) as u64
    };
    let mut open: std::collections::HashMap<(String, String), Vec<(u64, String)>> =
        Default::default();
    let mut out = vec![];
    for e in v["traceEvents"].as_array().into_iter().flatten() {
        let key = (str_of(e, "pid"), str_of(e, "tid"));
        match e["ph"].as_str() {
            Some("B") => open
                .entry(key)
                .or_default()
                .push((at(e), str_of(e, "name"))),
            Some("E") => {
                if let Some((start, text)) = open.get_mut(&key).and_then(Vec::pop) {
                    out.push(Ann {
                        pd: key.0,
                        row: key.1,
                        start,
                        end: at(e),
                        text,
                    });
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|a| a.start);
    Ok(out)
}

/// Runs a decoder stack over samples: one byte per sample for 8 channels, two (little-endian) for 16.
fn sigrok_run(
    samples: &[u8],
    channels: u8,
    rate: u64,
    stack: &[PdSpec],
) -> Result<Vec<Ann>, String> {
    use std::process::{Command, Stdio};
    let cli = sigrok_cli().ok_or("sigrok-cli isn't installed.")?;
    let input = format!("binary:numchannels={channels}:samplerate={rate}");
    let mut child = Command::new(cli)
        .args([
            "-I",
            &input,
            "-i",
            "-",
            "-P",
            &pd_arg(stack)?,
            "--protocol-decoder-jsontrace",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run sigrok-cli: {e}"))?;
    let mut stdin = child.stdin.take().unwrap();
    let out = std::thread::scope(|sc| {
        sc.spawn(move || stdin.write_all(samples)); // write while reading: neither pipe may fill up
        child.wait_with_output()
    })
    .map_err(|e| e.to_string())?;
    let err = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(err.trim().to_string());
    }
    let anns = parse_trace(&String::from_utf8_lossy(&out.stdout), rate as f64)?;
    match err.lines().find(|l| l.contains("Required channel")) {
        Some(l) if anns.is_empty() => Err(l.trim_start_matches("srd: ").to_string()),
        _ => Ok(anns),
    }
}

#[tauri::command]
async fn sigrok_decode(
    samples: Vec<u8>,
    channels: u8,
    rate: u64,
    stack: Vec<PdSpec>,
) -> Result<Vec<Ann>, String> {
    tauri::async_runtime::spawn_blocking(move || sigrok_run(&samples, channels, rate, &stack))
        .await
        .map_err(|e| e.to_string())?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Link::default())
        .manage(Live::default())
        .invoke_handler(tauri::generate_handler![
            list_ports,
            capture,
            autoset,
            live_start,
            live_stop,
            sigrok_decoders,
            sigrok_decode,
            write_text,
            read_text
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
            rle: false,
        }
    }

    const UART_SHOW: &str = "ID: uart
Name: UART
Long name: Universal Asynchronous Receiver/Transmitter
Description: Asynchronous, serial bus.
Possible decoder input IDs:
- logic
Possible decoder output IDs:
- uart
Annotation rows:
- rx-data-bits (RX bits): rx-data-bits
- rx-data (RX): rx-data, rx-start, rx-stop
Required channels:
None.
Optional channels:
- rx (RX): UART receive line
Options:
- baudrate: Baud rate (default 115200)
- parity: Parity ('none', 'odd', default 'none')
- rx_packet_delim: RX packet delimiter (decimal) (default -1)
";

    #[test]
    fn decoder_details() {
        let d = parse_show(UART_SHOW);
        assert_eq!((d.id.as_str(), d.name.as_str()), ("uart", "UART"));
        assert_eq!(
            (d.inputs, d.outputs),
            (vec!["logic".to_string()], vec!["uart".to_string()])
        );
        assert_eq!(d.rows, ["RX bits", "RX"]);
        assert!(d.required.is_empty());
        assert_eq!(
            d.optional,
            [PdChannel {
                id: "rx".into(),
                name: "RX".into(),
                desc: "UART receive line".into()
            }]
        );
        let o = |id: &str, desc: &str, default: &str, values: &[&str]| PdOption {
            id: id.into(),
            desc: desc.into(),
            default: default.into(),
            values: values.iter().map(|v| v.to_string()).collect(),
        };
        assert_eq!(
            d.options,
            [
                o("baudrate", "Baud rate", "115200", &[]),
                o("parity", "Parity", "none", &["none", "odd"]),
                o(
                    "rx_packet_delim",
                    "RX packet delimiter (decimal)",
                    "-1",
                    &[]
                ),
            ]
        );
    }

    #[test]
    fn decoder_without_rows() {
        let d = parse_show("ID: lm75\nAnnotation classes:\n- celsius: Temperature in degrees Celsius\n- warnings: Human-readable warnings\nAnnotation rows:\nNone.\n");
        assert_eq!(d.rows, ["celsius", "warnings"]);
    }

    #[test]
    fn decoder_stack_argument() {
        let pd = |id: &str, ch: &[(&str, u8)], opts: &[(&str, &str)]| PdSpec {
            id: id.into(),
            channels: ch.iter().map(|&(k, v)| (k.into(), v)).collect(),
            options: opts.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
        };
        let stack = [
            pd("i2c", &[("scl", 2), ("sda", 3)], &[]),
            pd("eeprom24xx", &[], &[("chip", "generic")]),
        ];
        assert_eq!(
            pd_arg(&stack).unwrap(),
            "i2c:scl=2:sda=3,eeprom24xx:chip=generic"
        );
        assert!(pd_arg(&[pd("uart", &[], &[("format", "a:b")])]).is_err());
        assert!(pd_arg(&[pd("uart,i2c", &[], &[])]).is_err());
        assert!(pd_arg(&[]).is_err());
    }

    #[test]
    fn decoder_trace() {
        let json = r#"{"traceEvents": [
{"ph": "B", "ts": 50.000000, "pid": "uart-1", "tid": "RX", "name": "Start bit"},
{"ph": "E", "ts": 59.000000, "pid": "uart-1", "tid": "RX", "name": "Start bit"},
{"ph": "B", "ts": 59.000000, "pid": "uart-1", "tid": "RX", "name": "48"},
{"ph": "E", "ts": 129.500000, "pid": "uart-1", "tid": "RX", "name": "48"}
]}"#;
        let a = parse_trace(json, 2e6).unwrap();
        let ann = |start, end, text: &str| Ann {
            pd: "uart-1".into(),
            row: "RX".into(),
            start,
            end,
            text: text.into(),
        };
        assert_eq!(a, [ann(100, 118, "Start bit"), ann(118, 259, "48")]);
        assert!(parse_trace("", 1e6).unwrap().is_empty());
        assert!(parse_trace("{oops", 1e6).is_err());
    }

    /// Writes the installed decoder catalog as JSON, for trying the UI in a browser:
    /// MOJO_CATALOG=path cargo test dump_catalog -- --ignored
    #[test]
    #[ignore]
    fn dump_catalog() {
        let c = tauri::async_runtime::block_on(sigrok_decoders()).unwrap();
        println!("{} decoders", c.len());
        std::fs::write(
            std::env::var("MOJO_CATALOG").unwrap(),
            serde_json::to_string(&c).unwrap(),
        )
        .unwrap();
    }

    /// The real thing, when sigrok-cli is installed: "Hi" at 115200 baud on D1, 1 MHz.
    #[test]
    fn sigrok_uart_roundtrip() {
        if sigrok_cli().is_none() {
            assert!(
                std::env::var_os("MOJO_NEED_SIGROK").is_none(),
                "sigrok-cli not found"
            );
            return;
        }
        let mut s = vec![0xFFu8; 50];
        for b in b"Hi" {
            let start = s.len() as f64;
            for k in 0..10 {
                let bit = k == 9 || (k > 0 && (b >> (k - 1)) & 1 == 1);
                let end = (start + (k + 1) as f64 * 1e6 / 115_200.0).round() as usize;
                s.resize(end, if bit { 0xFF } else { 0xFD });
            }
        }
        s.resize(s.len() + 50, 0xFF);
        let stack = [PdSpec {
            id: "uart".into(),
            channels: [("rx".to_string(), 1)].into(),
            options: [("format".to_string(), "ascii".to_string())].into(),
        }];
        let a = sigrok_run(&s, 8, 1_000_000, &stack).unwrap();
        let data: Vec<&str> = a
            .iter()
            .filter(|a| a.row == "RX" && a.text.len() == 1)
            .map(|a| a.text.as_str())
            .collect();
        assert_eq!(data, ["H", "i"]);
    }

    #[test]
    fn live_records() {
        let mut p = LiveParser::default();
        let mut out = vec![];
        // 'H' 30 us in; then 'i' on receiver 1 with a framing error 20 us later, fed split mid-record;
        // then 1000 steps of silence, 5 dropped bytes, and 'x'.
        p.feed(&[0x83, b'H', 0b1110_0010], &mut out);
        p.feed(&[b'i', 0x03, 0xE8, 0x7F, 5, 0x80, b'x'], &mut out);
        let r = |t_us, rx, byte, err, dropped| LiveRec {
            t_us,
            rx,
            byte,
            err,
            dropped,
        };
        assert_eq!(
            out,
            [
                r(30, 0, b'H', false, 0),
                r(50, 1, b'i', true, 0),
                r(10050, 0, 0, false, 5),
                r(10050, 0, b'x', false, 0),
            ]
        );
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
        // long capture: both bytes of each entry, plus flags bit 14
        let rle = setup(&Settings {
            rle: true,
            ..settings(1_000_000, 1024, 1024, 0, 0)
        });
        assert_eq!(&rle[0..5], &[0x82, 0x30, 0x40, 0, 0]);
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

    /// Long capture on the Pro Micro rig: 16K run-length entries at 1 MHz of a UART that ticks every
    /// 100 ms cover seconds, and D1 still toggles inside them.
    #[test]
    #[ignore]
    fn board_long_capture() {
        let s = Settings {
            rle: true,
            ..settings(1_000_000, 16384, 16384, 0, 0)
        };
        let e = board()
            .with(&port(), |p| capture_on(p, &s))
            .unwrap()
            .unwrap();
        assert_eq!(e.len(), 16384);
        let total: usize = e.iter().map(|&w| (w >> 8) as usize + 1).sum();
        let d1_changes = e.windows(2).filter(|w| (w[0] ^ w[1]) & 2 != 0).count();
        println!(
            "long capture: {} entries cover {:.2} s, D1 changes {d1_changes} times",
            e.len(),
            total as f64 / 1e6
        );
        assert!(total >= 2_000_000, "only {total} samples");
        assert!(d1_changes > 100);
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

    /// Live log of the Pro Micro's 115200-baud ticks on D1 for 3 s: every line arrives, in order,
    /// about 100 ms apart, nothing dropped.
    #[test]
    #[ignore]
    fn board_live_log() {
        let stop = AtomicBool::new(false);
        let (mut recs, t) = (vec![], Instant::now());
        board()
            .with(&port(), |p| {
                live_on(
                    p,
                    &[LiveRx {
                        ch: 1,
                        baud: 115_200,
                    }],
                    &stop,
                    |r| {
                        recs.extend(r);
                        if t.elapsed() > Duration::from_secs(3) {
                            stop.store(true, Ordering::Relaxed);
                        }
                    },
                )
            })
            .unwrap();
        assert!(recs.iter().all(|r| r.dropped == 0 && !r.err && r.rx == 0));
        let text: String = recs.iter().map(|r| r.byte as char).collect();
        let ticks: Vec<u32> = text
            .lines()
            .filter_map(|l| l.trim().strip_prefix("tick ")?.parse().ok())
            .collect();
        println!(
            "live log: {} bytes, ticks {:?}..{:?}",
            recs.len(),
            ticks.first(),
            ticks.last()
        );
        assert!(ticks.len() >= 25, "only {} ticks", ticks.len());
        assert!(
            ticks.windows(2).all(|w| w[1] == w[0] + 1),
            "ticks skipped: {ticks:?}"
        );
        let starts: Vec<u64> = recs
            .windows(2)
            .filter(|w| w[0].byte == b'\n')
            .map(|w| w[1].t_us)
            .collect();
        assert!(
            starts
                .windows(2)
                .all(|w| (90_000..110_000).contains(&(w[1] - w[0]))),
            "ticks not 100 ms apart"
        );
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
                rle: false,
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
