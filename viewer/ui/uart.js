// 8N1 UART decoding of captured samples (bit `ch` of each byte in `data`).
const UART_BAUDS = [300, 1200, 2400, 4800, 9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1000000, 2000000];

/** Baud rate from pulse widths: shortest pulse ≈ one bit, refined over all short pulses, snapped to a standard rate within 5%. */
function guessBaud(data, ch, rate) {
  const m = 1 << ch, runs = [];
  let last = -1, prev = data[0] & m;
  for (let i = 1; i < data.length; i++) {
    const b = data[i] & m;
    if (b !== prev) { if (last >= 0) runs.push(i - last); last = i; prev = b; }
  }
  if (!runs.length) return null;
  const est = Math.min(...runs);
  let total = 0, bits = 0;
  for (const r of runs) if (r <= est * 10) { total += r; bits += Math.round(r / est); }
  const raw = rate * bits / total;
  const std = UART_BAUDS.reduce((a, b) => Math.abs(b - raw) < Math.abs(a - raw) ? b : a);
  return Math.abs(std - raw) / std < 0.05 ? std : Math.round(raw);
}

/** Frames as {start, end} in samples, the byte, and ok=false on a framing error (stop bit low). */
function decodeUart(data, ch, rate, baud) {
  const m = 1 << ch, spb = rate / baud, n = data.length, out = [];
  const at = t => data[Math.min(n - 1, Math.round(t))] & m;
  let i = 1;
  while (i < n && !(data[i - 1] & m)) i++;          // wait for idle-high: skip a frame cut off at the start
  for (; i < n; i++) {
    if (!(data[i - 1] & m) || (data[i] & m)) continue; // falling edge = start bit
    const s = i - 0.5;
    if (s + spb * 9.5 >= n) break;                      // frame runs past the end of the capture
    if (at(s + spb * 0.5)) continue;                    // glitch, not a start bit
    let byte = 0;
    for (let k = 0; k < 8; k++) if (at(s + spb * (1.5 + k))) byte |= 1 << k;
    out.push({ start: s, end: s + spb * 10, byte, ok: !!at(s + spb * 9.5) });
    i = Math.floor(s + spb * 9.5);
  }
  return out;
}

if (typeof module !== 'undefined') module.exports = { guessBaud, decodeUart, UART_BAUDS };
