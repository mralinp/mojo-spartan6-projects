// node viewer/ui/sigrok.test.js
const assert = require('node:assert');
const { firstDecoders, stackable, newBus, shownRow, homeChannel, specOf, annItems } = require('./sigrok.js');

const ch = (id, name = id.toUpperCase()) => ({ id, name, desc: '' });
const catalog = [
  { id: 'eeprom24xx', name: '24xx EEPROM', inputs: ['i2c'], outputs: [], required: [], optional: [], options: [], rows: ['Fields', 'Warnings'] },
  { id: 'i2c', name: 'I²C', inputs: ['logic'], outputs: ['i2c'], required: [ch('scl'), ch('sda')], optional: [],
    options: [{ id: 'address_format', desc: 'Displayed slave address format', default: 'shifted', values: ['shifted', 'unshifted'] }],
    rows: ['Bits', 'Address/Data', 'Warnings'] },
  { id: 'uart', name: 'UART', inputs: ['logic'], outputs: ['uart'], required: [], optional: [ch('rx'), ch('tx')],
    options: [{ id: 'baudrate', desc: 'Baud rate', default: '115200', values: [] }], rows: ['RX bits', 'RX', 'RX warnings', 'TX bits', 'TX'] },
];

assert.deepStrictEqual(firstDecoders(catalog).map(d => d.id), ['i2c', 'uart']);
assert.deepStrictEqual(stackable(catalog, catalog[1]).map(d => d.id), ['eeprom24xx']);
assert.deepStrictEqual(stackable(catalog, catalog[2]), []);

// channels land on D0, D1, …; optional ones only while they fit
const u = newBus(catalog[2], 8);
assert.deepStrictEqual(u.stack[0].channels, { rx: 0, tx: 1 });
assert.deepStrictEqual(newBus(catalog[2], 1).stack[0].channels, {});
const i2c = newBus(catalog[1], 8);
assert.deepStrictEqual(i2c.stack[0].channels, { scl: 0, sda: 1 });
assert.strictEqual(homeChannel(i2c), 1);              // drawn under SDA, the data line
assert.strictEqual(homeChannel(u), 0);

// the default row skips bits; a stacked decoder's row wins; the user's pick wins over both
assert.deepStrictEqual(shownRow(u, catalog), { pd: 'uart', row: 'RX' });
i2c.stack.push({ id: 'eeprom24xx', channels: {}, options: {} });
assert.deepStrictEqual(shownRow(i2c, catalog), { pd: 'eeprom24xx', row: 'Fields' });
assert.deepStrictEqual(shownRow({ ...u, show: { pd: 'uart', row: 'TX' } }, catalog), { pd: 'uart', row: 'TX' });

// only options that differ from the default go to sigrok-cli
u.stack[0].options = { baudrate: '115200' };
assert.deepStrictEqual(specOf(u, catalog)[0].options, {});
u.stack[0].options = { baudrate: 9600, parity: '' };
assert.deepStrictEqual(specOf(u, catalog), [{ id: 'uart', channels: { rx: 0, tx: 1 }, options: { baudrate: '9600' } }]);

// items: the shown row plus warnings, under the home channel
const anns = [
  { pd: 'uart-1', row: 'RX bits', start: 10, end: 19, text: '1' },
  { pd: 'uart-1', row: 'RX', start: 9, end: 95, text: 'H' },
  { pd: 'uart-1', row: 'RX warnings', start: 95, end: 95, text: 'Frame error' },
  { pd: 'uart-1', row: 'TX', start: 9, end: 95, text: 'Hello' },
];
const it = annItems(anns, { ...u, show: null }, catalog);
assert.deepStrictEqual(it.map(i => [i.long, i.short, i.ch, i.bad, i.byte]), [['H', 'H', 0, false, null], ['Frame error', 'Fra…', 0, true, null]]);
assert.deepStrictEqual(annItems([{ pd: 'uart-1', row: 'RX', start: 0, end: 9, text: 'Address write: 3C' }], u, catalog)[0].short, '3C');
assert.strictEqual(it[1].end, 96);                    // zero-length annotations still get a sliver
assert.strictEqual(it[0].note, 'UART: H');
console.log('sigrok ok');
