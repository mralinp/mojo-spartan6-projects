// node viewer/ui/buses.test.js
const assert = require('node:assert');
const { decodeI2C, decodeSPI } = require('./buses.js');

// ---- I²C: SCL on bit 0, SDA on bit 1, 4 samples per quarter clock ----
function i2c(ops) {
  const s = [];
  let scl = 1, sda = 1;
  const put = (n = 4) => { for (let k = 0; k < n; k++) s.push(scl | sda << 1); };
  put(8);
  for (const op of ops) {
    if (op === 'S') { sda = 1; put(); scl = 1; put(); sda = 0; put(); scl = 0; put(); }      // (repeated) start
    else if (op === 'P') { sda = 0; put(); scl = 1; put(); sda = 1; put(8); }                // stop
    else for (const b of op) { sda = b; put(); scl = 1; put(2); scl = 0; put(); }            // bits, clocked
  }
  return Uint8Array.from(s);
}
const bits = (byte, ack) => [...Array(8)].map((_, k) => (byte >> (7 - k)) & 1).concat([ack ? 0 : 1]);

// write 0xA5, 0x00 to 0x3C, repeated start, read one byte 0x42 (master NACKs it), stop
const t = i2c(['S', bits(0x3C << 1, true), bits(0xA5, true), bits(0x00, true),
               'S', bits(0x3C << 1 | 1, true), bits(0x42, false), 'P']);
const ev = decodeI2C(t, 0, 1).map(e => e.type === 'addr' ? `A${e.addr.toString(16)}${e.read ? 'R' : 'W'}${e.ack ? '+' : '-'}`
  : e.type === 'data' ? `${e.byte.toString(16)}${e.ack ? '+' : '-'}` : e.type === 'start' ? 'S' : 'P');
assert.deepStrictEqual(ev, ['S', 'A3cW+', 'a5+', '0+', 'S', 'A3cR+', '42-', 'P']);
// an address nobody answers
assert.deepStrictEqual(decodeI2C(i2c(['S', bits(0x50 << 1, false), 'P']), 0, 1).filter(e => e.type === 'addr').map(e => e.ack), [false]);

// ---- SPI: SCLK bit 0, MOSI bit 1, MISO bit 2, CS bit 3 ----
function spi(mosiBytes, misoBytes, mode, msbFirst = true) {
  const cpol = mode >> 1, cpha = mode & 1, s = [];
  let sclk = cpol, cs = 1, mo = 0, mi = 0;
  const put = (n = 3) => { for (let k = 0; k < n; k++) s.push(sclk | mo << 1 | mi << 2 | cs << 3); };
  put(6); cs = 0; put();
  mosiBytes.forEach((m, j) => {
    for (let k = 0; k < 8; k++) {
      const sh = msbFirst ? 7 - k : k, setData = () => { mo = (m >> sh) & 1; mi = (misoBytes[j] >> sh) & 1; };
      if (cpha === 0) { setData(); put(); sclk ^= 1; put(); sclk ^= 1; put(); }   // data before the first edge
      else { sclk ^= 1; put(1); setData(); put(2); sclk ^= 1; put(); }            // data changes just after the first edge, read on the second
    }
  });
  put(); cs = 1; put(6);
  return Uint8Array.from(s);
}
for (const mode of [0, 1, 2, 3]) {
  const d = spi([0x9F, 0x00, 0x00], [0xFF, 0xEF, 0x40], mode);
  const got = decodeSPI(d, { sclk: 0, mosi: 1, miso: 2, cs: 3, mode });
  assert.deepStrictEqual(got.map(b => [b.mosi, b.miso]), [[0x9F, 0xFF], [0x00, 0xEF], [0x00, 0x40]], `mode ${mode}`);
}
// LSB first, MISO not wired
const lsb = decodeSPI(spi([0x01, 0x80], [0, 0], 0, false), { sclk: 0, mosi: 1, cs: 3, msbFirst: false });
assert.deepStrictEqual(lsb.map(b => [b.mosi, b.miso]), [[0x01, null], [0x80, null]]);
// wrong mode reads garbage, not the bytes: the mode matters
assert.notDeepStrictEqual(decodeSPI(spi([0x9F], [0], 1), { sclk: 0, mosi: 1, cs: 3, mode: 0 }).map(b => b.mosi), [0x9F]);
console.log('i2c and spi ok');
