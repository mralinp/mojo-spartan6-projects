// Live UART log, Wireshark-style: bytes from the analyzer's receivers ({tUs, rx, byte, err, dropped})
// grouped into messages. A message ends at a newline, after an idle gap, or when the other line talks.

const GAP_US = 2000;                      // an idle line this long ends a message

class LiveLog {
  constructor(gapUs = GAP_US) { this.gapUs = gapUs; this.msgs = []; this.open = [null, null]; this.lastUs = [0, 0]; }

  /** Adds records; returns the messages that are new or grew, in order. */
  push(recs) {
    const touched = new Set();
    for (const r of recs) {
      if (r.dropped) {
        this.open = [null, null];
        const m = { no: this.msgs.length + 1, tUs: r.tUs, rx: null, bytes: [], times: [], errs: 0, dropped: r.dropped };
        this.msgs.push(m); touched.add(m);
        continue;
      }
      let m = this.open[r.rx];
      if (m && (r.tUs - this.lastUs[r.rx] > this.gapUs || this.lastRx !== r.rx)) m = null;
      if (!m) {
        m = { no: this.msgs.length + 1, tUs: r.tUs, rx: r.rx, bytes: [], times: [], errs: 0, dropped: 0 };
        this.msgs.push(m);
        this.open[r.rx] = m;
      }
      m.bytes.push(r.byte); m.times.push(r.tUs); if (r.err) m.errs++;
      this.lastUs[r.rx] = r.tUs; this.lastRx = r.rx;
      if (r.byte === 0x0A) this.open[r.rx] = null;
      touched.add(m);
    }
    return [...touched];
  }
}

const asciiOf = bytes => bytes.map(b => b === 0x0A ? '↵' : b === 0x0D ? '' : b >= 0x20 && b < 0x7F ? String.fromCharCode(b) : '·').join('');
const hexOf = bytes => bytes.map(b => b.toString(16).padStart(2, '0')).join(' ');

/** Filter: words must all match the line name, the text or the hex ("tick", "rx", "0d 0a"). */
function matches(m, query, lineName) {
  const hay = `${m.dropped ? 'dropped' : lineName(m.rx)} ${asciiOf(m.bytes)} ${hexOf(m.bytes)}`.toLowerCase();
  return query.toLowerCase().split(/\s+/).filter(Boolean).every(w => hay.includes(w));
}

/** CSV for spreadsheets: one row per message. */
function logCSV(msgs, lineName) {
  const q = s => `"${String(s).replace(/"/g, '""')}"`;
  return ['no,time_s,line,length,errors,dropped,ascii,hex',
    ...msgs.map(m => [m.no, (m.tUs / 1e6).toFixed(5), q(m.dropped ? '' : lineName(m.rx)), m.bytes.length, m.errs, m.dropped,
      q(asciiOf(m.bytes)), q(hexOf(m.bytes))].join(','))].join('\n') + '\n';
}

if (typeof module !== 'undefined') module.exports = { LiveLog, asciiOf, hexOf, matches, logCSV };
