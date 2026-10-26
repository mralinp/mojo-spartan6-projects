// Comparing a capture against a reference one. A capture here: {samples, rate, trigIdx}.
// Both are aligned on an anchor sample (the trigger, else the first edge) and compared in time,
// so the two may have different sample rates.

/** The sample both captures are lined up on: the trigger, else the first edge, else 0. */
function anchor(c) {
  if (c.trigIdx !== null && c.trigIdx !== undefined) return c.trigIdx;
  for (let i = 1; i < c.samples.length; i++) if (c.samples[i] !== c.samples[i - 1]) return i;
  return 0;
}

/** Reference sample index at current sample index i (fractional; caller rounds). */
const refIndexAt = (cur, ref, i) => anchor(ref) + (i - anchor(cur)) * ref.rate / cur.rate;

/** Where one channel disagrees with the reference, in current-capture samples. Spans shorter
 *  than `tol` samples are edge jitter, not differences. Only the time both captures cover counts. */
function diffChannel(cur, ref, ch, tol = 2) {
  const a0 = anchor(cur), r0 = anchor(ref), k = ref.rate / cur.rate;
  const from = Math.max(0, Math.ceil(a0 - r0 / k)), to = Math.min(cur.samples.length, Math.floor(a0 + (ref.samples.length - 1 - r0) / k) + 1);
  const spans = [];
  let start = -1;
  for (let i = from; i < to; i++) {
    const j = Math.round(r0 + (i - a0) * k);
    const differs = ((cur.samples[i] ^ ref.samples[j]) >> ch) & 1;
    if (differs && start < 0) start = i;
    if (!differs && start >= 0) { if (i - start > tol) spans.push({ start, end: i }); start = -1; }
  }
  if (start >= 0 && to - start > tol) spans.push({ start, end: to });
  return { spans, from, to, longest: spans.reduce((m, s) => Math.max(m, s.end - s.start), 0) };
}

/** Decoded bytes, position by position: which differ, and how many one side has extra. */
function diffBytes(cur, ref) {
  const changed = [];
  for (let i = 0; i < Math.min(cur.length, ref.length); i++) if (cur[i] !== ref[i]) changed.push({ i, cur: cur[i], ref: ref[i] });
  return { changed, extra: cur.length - ref.length };
}

if (typeof module !== 'undefined') module.exports = { anchor, refIndexAt, diffChannel, diffBytes };
