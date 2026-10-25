// node viewer/ui/measure.test.js
const assert = require('node:assert');
const { measure, nearestEdge } = require('./measure.js');

// D0: square wave, period 10, high 3 (30% duty), starting mid-low so the first pulse is cut off.
// D1: stays high. D2: stays low.
const n = 105, d = new Uint8Array(n);
for (let i = 0; i < n; i++) d[i] = ((i + 5) % 10 < 3 ? 1 : 0) | 2;

const sq = measure(d, 0);
assert.strictEqual(sq.rising, 10);
assert.strictEqual(sq.falling, 10);
assert.deepStrictEqual([sq.period.min, sq.period.max, sq.period.mean], [10, 10, 10]);
assert.ok(Math.abs(sq.duty - 0.3) < 1e-9, `duty ${sq.duty}`);
assert.deepStrictEqual([sq.high.min, sq.high.max], [3, 3]);
assert.deepStrictEqual([sq.low.min, sq.low.max], [7, 7]);
assert.strictEqual(sq.level, null);

assert.strictEqual(measure(d, 1).level, 1);
assert.strictEqual(measure(d, 2).level, 0);
assert.strictEqual(measure(d, 2).period, null);

// A window: only the pulses wholly inside it count.
const w = measure(d, 0, 20, 46); // rising edges at 25, 35, 45: two whole periods
assert.strictEqual(w.period.n, 2);
assert.deepStrictEqual([w.high.min, w.high.max], [3, 3]);

// Uneven pulses: min/mean/max differ.
const u = new Uint8Array([0, 1, 0, 0, 1, 1, 1, 0, 0, 0, 0, 1, 1, 0]);
const um = measure(u, 0);
assert.deepStrictEqual([um.high.min, um.high.max, um.high.n], [1, 3, 3]);
assert.strictEqual(um.high.mean, (1 + 3 + 2) / 3);

// Marker snapping
assert.strictEqual(nearestEdge([5, 20, 40], 22, 3), 20);
assert.strictEqual(nearestEdge([5, 20, 40], 30, 3), null);
assert.strictEqual(nearestEdge([], 3, 10), null);
console.log('measurements ok');
