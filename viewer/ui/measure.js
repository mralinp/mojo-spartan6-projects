// Per-channel measurements over samples [i0, i1) of a capture (bit `ch` of each byte in `data`).
// Times are in samples; divide by the sample rate for seconds. Only whole pulses count:
// a pulse cut off by the window's ends isn't a real width.

const stats = xs => xs.length
  ? { min: Math.min(...xs), max: Math.max(...xs), mean: xs.reduce((a, b) => a + b, 0) / xs.length, n: xs.length }
  : null;

function measure(data, ch, i0 = 0, i1 = data.length) {
  const m = 1 << ch, bit = i => (data[i] & m) ? 1 : 0;
  i0 = Math.max(0, Math.floor(i0)); i1 = Math.min(data.length, Math.ceil(i1));
  const rising = [], falling = [], highs = [], lows = [];
  let last = -1, high = 0;
  for (let i = i0; i < i1; i++) {
    high += bit(i);
    if (i > i0 && bit(i) !== bit(i - 1)) {
      (bit(i) ? rising : falling).push(i);
      if (last >= 0) (bit(i) ? lows : highs).push(i - last); // the pulse that just ended
      last = i;
    }
  }
  const n = i1 - i0;
  const out = { edges: rising.length + falling.length, rising: rising.length, falling: falling.length,
    level: high === n ? 1 : high === 0 ? 0 : null, high: stats(highs), low: stats(lows), period: null, duty: null };
  // Period and duty from whole cycles: rising edge to rising edge.
  if (rising.length >= 2) {
    const periods = rising.slice(1).map((r, j) => r - rising[j]);
    out.period = stats(periods);
    let hi = 0;
    for (let i = rising[0]; i < rising[rising.length - 1]; i++) hi += bit(i);
    out.duty = hi / (rising[rising.length - 1] - rising[0]);
  }
  return out;
}

/** Index of the edge (on any channel) nearest to sample `i`, or null if none within `tol` samples. */
function nearestEdge(edges, i, tol) {
  let lo = 0, hi = edges.length;
  while (lo < hi) { const mid = (lo + hi) >> 1; if (edges[mid] < i) lo = mid + 1; else hi = mid; }
  let best = null;
  for (const j of [lo - 1, lo]) if (j >= 0 && j < edges.length && Math.abs(edges[j] - i) <= tol && (best === null || Math.abs(edges[j] - i) < Math.abs(best - i))) best = edges[j];
  return best;
}

if (typeof module !== 'undefined') module.exports = { measure, nearestEdge };
