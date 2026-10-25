// Capture files (.mojola) and VCD / CSV export. Pure functions, so Node can test them.
// A capture: {samples: Uint8Array, rate, trigIdx (or null), takenAt (ISO time), names[8],
//             uartOn[8], baud (0 = auto), buses}

const CAPTURE_FORMAT = 'mojo-la-capture';

function toB64(u8) {
  let s = '';
  for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
  return btoa(s);
}
function fromB64(b) {
  const s = atob(b), u = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
  return u;
}

function serializeCapture(c) {
  return JSON.stringify({
    format: CAPTURE_FORMAT, version: 1, takenAt: c.takenAt, rate: c.rate, trigIdx: c.trigIdx,
    names: c.names, uartOn: c.uartOn, baud: c.baud, buses: c.buses, samples: toB64(c.samples),
  }, null, 1);
}

/** Throws an Error with a readable message when the file isn't a capture we can open. */
function parseCapture(text) {
  let o;
  try { o = JSON.parse(text); } catch { throw new Error('This file isn\'t a Mojo LA capture (it isn\'t JSON).'); }
  if (o?.format !== CAPTURE_FORMAT) throw new Error('This file isn\'t a Mojo LA capture.');
  if (o.version > 1) throw new Error(`This capture was saved by a newer Mojo LA (format ${o.version}). Update the app to open it.`);
  const samples = fromB64(o.samples || '');
  if (!samples.length || !(o.rate > 0)) throw new Error('This capture file is damaged: it has no samples or no sample rate.');
  const eight = (a, fill) => Array.from({ length: 8 }, (_, i) => Array.isArray(a) && a[i] !== undefined ? a[i] : fill);
  return {
    samples, rate: o.rate, takenAt: o.takenAt || null,
    trigIdx: Number.isInteger(o.trigIdx) && o.trigIdx >= 0 && o.trigIdx < samples.length ? o.trigIdx : null,
    names: eight(o.names, null), uartOn: eight(o.uartOn, false).map(Boolean),
    baud: +o.baud || 0, buses: Array.isArray(o.buses) ? o.buses : [],
  };
}

const vcdName = s => String(s).replace(/[^A-Za-z0-9_]+/g, '_') || 'ch';

/** VCD with a 1 ns timescale (every rate divides 1 GHz). `label(ch)` names each wire. */
function toVCD(c, label) {
  const per = Math.round(1e9 / c.rate), s = c.samples, ids = 'ABCDEFGH';
  const out = [
    `$date ${c.takenAt || new Date().toISOString()} $end`,
    '$version Mojo LA $end',
    `$comment ${s.length} samples at ${c.rate} Hz${c.trigIdx !== null ? `, trigger at sample ${c.trigIdx}` : ''} $end`,
    '$timescale 1ns $end',
    '$scope module mojo_la $end',
  ];
  for (let ch = 0; ch < 8; ch++) out.push(`$var wire 1 ${ids[ch]} ${vcdName(label(ch))} $end`);
  out.push('$upscope $end', '$enddefinitions $end', '#0', '$dumpvars');
  for (let ch = 0; ch < 8; ch++) out.push(`${(s[0] >> ch) & 1}${ids[ch]}`);
  out.push('$end');
  for (let i = 1; i < s.length; i++) {
    const diff = s[i] ^ s[i - 1];
    if (!diff) continue;
    out.push(`#${i * per}`);
    for (let ch = 0; ch < 8; ch++) if ((diff >> ch) & 1) out.push(`${(s[i] >> ch) & 1}${ids[ch]}`);
  }
  out.push(`#${s.length * per}`);
  return out.join('\n') + '\n';
}

const csvField = v => /[",\n]/.test(v) ? `"${String(v).replace(/"/g, '""')}"` : String(v);

/** One row per sample: time in seconds from the trigger (or the first sample), then each channel's 0/1. */
function toCSV(c, label) {
  const s = c.samples, t0 = c.trigIdx ?? 0;
  const rows = [['time_s', ...Array.from({ length: 8 }, (_, ch) => label(ch))].map(csvField).join(',')];
  for (let i = 0; i < s.length; i++) {
    let row = ((i - t0) / c.rate).toFixed(9);
    for (let ch = 0; ch < 8; ch++) row += ',' + ((s[i] >> ch) & 1);
    rows.push(row);
  }
  return rows.join('\n') + '\n';
}

if (typeof module !== 'undefined') module.exports = { serializeCapture, parseCapture, toVCD, toCSV };
