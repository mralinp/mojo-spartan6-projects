// I²C and SPI decoding of captured samples. Channels are bit numbers in each sample byte;
// positions are sample indexes.

/** I²C on SCL/SDA. Events, in order:
 *  {type: 'start' | 'stop', at}
 *  {type: 'addr', start, end, addr, read, ack}   first byte after a (repeated) start
 *  {type: 'data', start, end, byte, ack}
 *  Bits are read on SCL rising edges; start/stop are SDA changing while SCL is high.
 *  ponytail: 7-bit addresses only; a 10-bit address shows as an address byte plus a data byte. */
function decodeI2C(data, scl, sda) {
  const C = 1 << scl, D = 1 << sda, out = [];
  let bits = -1, val = 0, first = 0, addrNext = false;
  for (let i = 1; i < data.length; i++) {
    const c0 = data[i - 1] & C, c1 = data[i] & C, d0 = data[i - 1] & D, d1 = data[i] & D;
    if (c0 && c1 && d0 && !d1) { out.push({ type: 'start', at: i }); bits = 0; val = 0; addrNext = true; continue; }
    if (c0 && c1 && !d0 && d1) { out.push({ type: 'stop', at: i }); bits = -1; continue; }
    if (bits < 0 || c0 || !c1) continue;            // only SCL rising edges inside a transfer
    const bit = d1 ? 1 : 0;
    if (bits < 8) {
      if (bits === 0) first = i;
      val = val << 1 | bit;
      bits++;
    } else {                                        // 9th clock: SDA low is ACK
      const ack = !bit;
      out.push(addrNext
        ? { type: 'addr', start: first, end: i, addr: val >> 1, read: !!(val & 1), ack }
        : { type: 'data', start: first, end: i, byte: val, ack });
      addrNext = false; bits = 0; val = 0;
    }
  }
  return out;
}

/** SPI bytes: [{start, end, mosi, miso}], null for a line that isn't wired (channel < 0).
 *  mode = CPOL * 2 + CPHA: bits are read on rising SCLK in modes 0 and 3, falling in 1 and 2.
 *  With a chip select, each select starts a new byte; without one, bytes count from the first clock.
 *  ponytail: 8-bit words only. */
function decodeSPI(data, { sclk, mosi = -1, miso = -1, cs = -1, mode = 0, msbFirst = true, csActiveLow = true }) {
  const K = 1 << sclk, readOnRise = ((mode >> 1) ^ (mode & 1)) === 0, out = [];
  const selected = i => cs < 0 || ((data[i] >> cs) & 1) === (csActiveLow ? 0 : 1);
  const bit = (i, ch) => (data[i] >> ch) & 1;
  let n = 0, mo = 0, mi = 0, first = 0;
  for (let i = 1; i < data.length; i++) {
    if (selected(i) !== selected(i - 1)) { n = 0; mo = mi = 0; }  // select edge: frame boundary
    if (!selected(i)) continue;
    const k0 = !!(data[i - 1] & K), k1 = !!(data[i] & K);
    if (k0 === k1 || k1 !== readOnRise) continue;
    if (n === 0) first = i;
    if (msbFirst) {
      if (mosi >= 0) mo = mo << 1 | bit(i, mosi);
      if (miso >= 0) mi = mi << 1 | bit(i, miso);
    } else {
      if (mosi >= 0) mo |= bit(i, mosi) << n;
      if (miso >= 0) mi |= bit(i, miso) << n;
    }
    if (++n === 8) {
      out.push({ start: first, end: i, mosi: mosi >= 0 ? mo : null, miso: miso >= 0 ? mi : null });
      n = 0; mo = mi = 0;
    }
  }
  return out;
}

if (typeof module !== 'undefined') module.exports = { decodeI2C, decodeSPI };
