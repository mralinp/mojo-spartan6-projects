// node viewer/ui/compare.test.js
const assert = require('node:assert');
const { anchor, refIndexAt, diffChannel, diffBytes } = require('./compare.js');

// A clock on D0 with period 20, starting high at `phase`; D1 has a pulse at `pulse`.
function cap(n, { phase = 0, pulse = 50, width = 10, trig = null } = {}) {
  const s = new Uint8Array(n);
  for (let i = 0; i < n; i++) s[i] = ((i + phase) % 20 < 10 ? 1 : 0) | (i >= pulse && i < pulse + width ? 2 : 0);
  return { samples: s, rate: 1e6, trigIdx: trig };
}

// identical captures: no differences
const a = cap(200, { trig: 40 });
assert.deepStrictEqual(diffChannel(a, a, 0).spans, []);
assert.deepStrictEqual(diffChannel(a, a, 1).spans, []);

// same signal, triggered at different points in the buffer: aligned on the trigger, nothing differs
const shifted = { ...cap(200, { phase: 10, pulse: 60, trig: 50 }) };
assert.strictEqual(anchor(shifted), 50);
assert.deepStrictEqual(diffChannel(a, shifted, 1).spans, []);
assert.deepStrictEqual(diffChannel(a, shifted, 0).spans, []);

// the D1 pulse 3 samples wider in the reference: one span of 3, found with tol 2, ignored with tol 3
const wider = cap(200, { width: 13, trig: 40 });
assert.deepStrictEqual(diffChannel(a, wider, 1, 2).spans, [{ start: 60, end: 63 }]);
assert.deepStrictEqual(diffChannel(a, wider, 1, 3).spans, []);
assert.strictEqual(diffChannel(a, wider, 1, 2).longest, 3);

// a reference at half the rate lines up in time
const half = { samples: a.samples.filter((_, i) => i % 2 === 0), rate: 5e5, trigIdx: 20 };
assert.strictEqual(refIndexAt(a, half, 40), 20);
assert.strictEqual(refIndexAt(a, half, 60), 30);
assert.deepStrictEqual(diffChannel(a, half, 1, 2).spans, []);

// only the overlap counts: a short reference covers samples 30..79 of the current one
const short = { samples: a.samples.slice(30, 80), rate: 1e6, trigIdx: 10 };
const d = diffChannel(a, short, 0);
assert.deepStrictEqual([d.from, d.to, d.spans.length], [30, 80, 0]);

// no trigger: aligned on the first edge
assert.strictEqual(anchor({ samples: Uint8Array.from([0, 0, 0, 4, 4]), trigIdx: null }), 3);

// decoded bytes
assert.deepStrictEqual(diffBytes([1, 2, 3], [1, 9, 3, 4]), { changed: [{ i: 1, cur: 2, ref: 9 }], extra: -1 });
assert.deepStrictEqual(diffBytes([], []), { changed: [], extra: 0 });
console.log('compare ok');
