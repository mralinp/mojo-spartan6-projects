// node viewer/ui/files.test.js
const assert = require('node:assert');
const { serializeCapture, parseCapture, toVCD, toCSV } = require('./files.js');

const samples = new Uint8Array(40000);
for (let i = 0; i < samples.length; i++) samples[i] = (i * 7) & 0xff;
const cap = {
  samples, rate: 1e6, trigIdx: 100, takenAt: '2026-10-26T01:02:03.000Z',
  names: ['TX', 'RX', null, 'SDA', null, null, null, 'CS'], uartOn: [true, true, false, false, false, false, false, false],
  baud: 115200, buses: [{ type: 'i2c', scl: 2, sda: 3 }],
};

// round trip, including more than one 32K chunk of base64 input
const back = parseCapture(serializeCapture(cap));
assert.deepStrictEqual(back.samples, samples);
for (const k of ['rate', 'trigIdx', 'takenAt', 'names', 'uartOn', 'baud', 'buses']) assert.deepStrictEqual(back[k], cap[k], k);

// readable errors for files we can't open
assert.throws(() => parseCapture('not json'), /isn't JSON/);
assert.throws(() => parseCapture('{"format":"other"}'), /isn't a Mojo LA capture/);
assert.throws(() => parseCapture(JSON.stringify({ format: 'mojo-la-capture', version: 9 })), /newer Mojo LA/);
assert.throws(() => parseCapture(JSON.stringify({ format: 'mojo-la-capture', version: 1, rate: 1e6, samples: '' })), /damaged/);
// a trigger outside the samples is dropped, missing fields get defaults
const loose = parseCapture(JSON.stringify({ format: 'mojo-la-capture', version: 1, rate: 1e6, samples: 'AAE=', trigIdx: 99 }));
assert.strictEqual(loose.trigIdx, null);
assert.deepStrictEqual(loose.uartOn, Array(8).fill(false));

// VCD: 4 samples at 200 MHz (5 ns), D0 toggling, D1 high from sample 2
const label = ch => cap.names[ch] || `D${ch}`;
const vcd = toVCD({ samples: Uint8Array.from([0, 1, 2, 3]), rate: 200e6, trigIdx: null, takenAt: 'now' }, label).split('\n');
assert.ok(vcd.includes('$timescale 1ns $end'));
assert.ok(vcd.includes('$var wire 1 A TX $end') && vcd.includes('$var wire 1 C D2 $end'));
const body = vcd.slice(vcd.indexOf('$dumpvars'));
assert.deepStrictEqual(body.slice(0, 10), ['$dumpvars', '0A', '0B', '0C', '0D', '0E', '0F', '0G', '0H', '$end']);
assert.deepStrictEqual(body.slice(10), ['#5', '1A', '#10', '0A', '1B', '#15', '1A', '#20', '']);
// names that aren't VCD identifiers are made safe
assert.ok(toVCD({ samples: Uint8Array.from([0]), rate: 1e6, trigIdx: null }, () => 'my bus, ch 1').includes('my_bus_ch_1'));

// CSV: header with quoted names, time relative to the trigger
const csv = toCSV({ samples: Uint8Array.from([1, 2]), rate: 1e6, trigIdx: 1 }, ch => ch === 0 ? 'a,b' : `D${ch}`).split('\n');
assert.strictEqual(csv[0], 'time_s,"a,b",D1,D2,D3,D4,D5,D6,D7');
assert.strictEqual(csv[1], '-0.000001000,1,0,0,0,0,0,0,0');
assert.strictEqual(csv[2], '0.000000000,0,1,0,0,0,0,0,0');
console.log('files ok');
