// sigrok decoders (libsigrokdecode, run by the app through sigrok-cli). A sigrok bus:
//   {type: 'sigrok', stack: [{id, channels: {name: ch}, options: {id: value}}], show: {pd, row}}
// The first decoder reads channels, each next one reads the decoder before it (I²C → 24xx EEPROM).
// catalog: the app's list of installed decoders, {id, name, longname, desc, inputs, outputs,
// required, optional, options, rows}.

const byId = (catalog, id) => catalog?.find(d => d.id === id);

/** Decoders that read logic lines: the ones a new bus can start with. */
const firstDecoders = catalog => (catalog || []).filter(d => d.inputs.includes('logic'));

/** Decoders that can go on top of `d`. */
const stackable = (catalog, d) => (catalog || []).filter(x => d && x.inputs.some(i => d.outputs.includes(i)));

/** A new bus for decoder `d`: its channels on D0, D1, … (optional ones too, while they fit in `nch`). */
function newBus(d, nch) {
  const chans = d.required.length + d.optional.length <= nch ? [...d.required, ...d.optional] : d.required;
  const channels = {};
  chans.forEach((c, k) => { if (k < nch) channels[c.id] = k; });
  return { type: 'sigrok', stack: [{ id: d.id, channels, options: {} }], show: null };
}

/** The row drawn and listed: the user's pick, else the top decoder's first row that isn't bits or warnings. */
function shownRow(bus, catalog) {
  if (bus.show) return bus.show;
  for (let k = bus.stack.length - 1; k >= 0; k--) {
    const d = byId(catalog, bus.stack[k].id);
    const row = d?.rows.find(r => !/bit|warn/i.test(r)) ?? d?.rows[0];
    if (row) return { pd: bus.stack[k].id, row };
  }
  return null;
}

/** The channel a bus's annotations are drawn under: its first wired line, data lines first. */
function homeChannel(bus) {
  const ch = bus.stack[0]?.channels || {};
  const pref = Object.keys(ch).find(k => /^(rx|tx|sda|mosi|miso|data|dq|d0|io)/i.test(k)) ?? Object.keys(ch)[0];
  return pref === undefined ? -1 : ch[pref];
}

/** What sigrok-cli is asked to run: only options that differ from their defaults. */
function specOf(bus, catalog) {
  return bus.stack.map(p => {
    const d = byId(catalog, p.id);
    const options = {};
    for (const [k, v] of Object.entries(p.options || {})) {
      const o = d?.options.find(o => o.id === k);
      if (v !== '' && (!o || String(v) !== o.default)) options[k] = String(v);
    }
    return { id: p.id, channels: p.channels, options };
  });
}

const pdOf = ann => ann.pd.replace(/-\d+$/, '');   // "uart-1" -> "uart"

/** A label for a narrow box: the value after "Address write: 3C", else the start of the text. */
function shortText(t) {
  const v = t.split(': ').at(-1);
  return v.length <= 6 ? v : t.slice(0, 3) + '…';
}

/** Annotations as viewer items: the shown row, plus any warnings (flagged bad) from the whole stack. */
function annItems(anns, bus, catalog) {
  const show = shownRow(bus, catalog), ch = homeChannel(bus), out = [];
  for (const a of anns) {
    const shown = show && pdOf(a) === show.pd && a.row === show.row;
    const warn = /warn|error/i.test(a.row);
    if (!shown && !warn) continue;
    const name = byId(catalog, pdOf(a))?.name ?? pdOf(a);
    out.push({ start: a.start, end: Math.max(a.end, a.start + 1), ch, byte: null, long: a.text,
      short: shortText(a.text), chr: '', note: `${name}: ${a.text}`, bad: warn, listed: true });
  }
  return out;
}

if (typeof module !== 'undefined') module.exports = { firstDecoders, stackable, newBus, shownRow, homeChannel, specOf, annItems };
