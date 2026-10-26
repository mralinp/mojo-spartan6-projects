// node viewer/ui/rle.test.js
const assert = require('node:assert');
const { expandRLE } = require('./rle.js');

const e = (len, value) => (len - 1) << 8 | value;

// runs of 3, 1 and 256 samples
const a = expandRLE([e(3, 0x02), e(1, 0x00), e(256, 0x02)]);
assert.strictEqual(a.samples.length, 260);
assert.deepStrictEqual(Array.from(a.samples.slice(0, 5)), [2, 2, 2, 0, 2]);
assert.ok(a.samples.slice(4).every(v => v === 2));
assert.strictEqual(a.trigIdx, null);

// triggered: the last 2 entries came after the trigger, so it sits where the second-to-last run starts
const t = expandRLE([e(10, 1), e(5, 0), e(7, 1), e(2, 0)], 2);
assert.strictEqual(t.trigIdx, 15);
assert.strictEqual(t.samples[15], 1);
assert.strictEqual(t.samples[14], 0);

// a long idle line costs one entry per 256 samples: 16K entries hold 4M samples
const idle = expandRLE(new Array(16384).fill(e(256, 0x03)));
assert.strictEqual(idle.samples.length, 16384 * 256);
assert.strictEqual(expandRLE([]).samples.length, 0);
console.log('rle ok');
