// node viewer/ui/uart.test.js
const assert = require('node:assert');
const { guessBaud, decodeUart } = require('./uart.js');

// Synthesize 8N1 on bit `ch` at `baud`, sampled at `rate`, with idle gaps between bytes.
function synth(bytes, ch, rate, baud, { badStop = -1 } = {}) {
  const spb = rate / baud, bits = [];
  for (let k = 0; k < 20; k++) bits.push(1);
  bytes.forEach((b, j) => {
    bits.push(0);
    for (let k = 0; k < 8; k++) bits.push((b >> k) & 1);
    bits.push(j === badStop ? 0 : 1);
    for (let k = 0; k < 3; k++) bits.push(1);
  });
  const n = Math.ceil(bits.length * spb), d = new Uint8Array(n);
  for (let i = 0; i < n; i++) d[i] = bits[Math.floor(i / spb)] << ch;
  return d;
}

const msg = [...Buffer.from('Hi Mojo!\r\n'), 0x00, 0xff, 0x55];
for (const [rate, baud] of [[1e6, 115200], [1e6, 9600], [10e6, 921600], [2e6, 230400]]) {
  const d = synth(msg, 1, rate, baud);
  assert.strictEqual(guessBaud(d, 1, rate), baud, `baud at ${rate}/${baud}`);
  const f = decodeUart(d, 1, rate, baud);
  assert.deepStrictEqual(f.map(x => x.byte), msg, `bytes at ${rate}/${baud}`);
  assert.ok(f.every(x => x.ok));
}
const bad = decodeUart(synth([0x41, 0x42], 0, 1e6, 115200, { badStop: 0 }), 0, 1e6, 115200);
assert.strictEqual(bad[0].ok, false, 'framing error flagged');
assert.strictEqual(guessBaud(new Uint8Array(100), 0, 1e6), null, 'flat line has no baud');
console.log('uart decoder ok');
