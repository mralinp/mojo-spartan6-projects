// node viewer/ui/livelog.test.js
const assert = require('node:assert');
const { LiveLog, asciiOf, hexOf, matches, logCSV } = require('./livelog.js');

const recs = (rx, t0, text, step = 87) => [...text].map((c, i) => ({ tUs: t0 + i * step, rx, byte: c.charCodeAt(0), err: false, dropped: 0 }));
const log = new LiveLog();

// a line per newline; a reply on the other receiver is its own message
log.push([...recs(0, 0, 'tick 1\r\ntick 2\r\n'), ...recs(1, 5000, 'OK\r\n')]);
assert.deepStrictEqual(log.msgs.map(m => [m.no, m.rx, asciiOf(m.bytes)]), [[1, 0, 'tick 1↵'], [2, 0, 'tick 2↵'], [3, 1, 'OK↵']]);
assert.strictEqual(log.msgs[1].tUs, 8 * 87);

// a message split across batches grows in place; an idle gap ends it
let t = log.push(recs(0, 100000, 'AT'));
t = log.push(recs(0, 100000 + 2 * 87, '+GMR'));
assert.strictEqual(t.length, 1); assert.strictEqual(asciiOf(t[0].bytes), 'AT+GMR');
log.push(recs(0, 200000, 'x'));
assert.strictEqual(asciiOf(log.msgs.at(-1).bytes), 'x');

// errors and drops
log.push([{ tUs: 300000, rx: 1, byte: 0, err: true, dropped: 0 }, { tUs: 300100, rx: 0, byte: 0, err: false, dropped: 7 }]);
assert.strictEqual(log.msgs.at(-2).errs, 1);
assert.strictEqual(log.msgs.at(-1).dropped, 7);

assert.strictEqual(hexOf([0x41, 0x0d, 0x0a]), '41 0d 0a');
const name = rx => ['TX', 'RX'][rx];
assert.ok(matches(log.msgs[2], 'rx ok', name));
assert.ok(matches(log.msgs[0], '0d 0a', name));
assert.ok(!matches(log.msgs[0], 'rx', name));
assert.ok(matches(log.msgs.at(-1), 'dropped', name));
const csv = logCSV(log.msgs.slice(0, 1), name).split('\n');
assert.strictEqual(csv[1], '1,0.00000,"TX",8,0,0,"tick 1↵","74 69 63 6b 20 31 0d 0a"');
console.log('livelog ok');
