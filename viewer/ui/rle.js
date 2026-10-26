// Long captures: the analyzer stores {run length - 1 (high byte), value (low byte)} entries, one per
// run of equal samples, oldest first after the backend reverses them. Expanding them gives plain samples.

/** entries: 16-bit words, oldest first. post: how many entries came after the trigger (0 = untriggered).
 *  The run that starts at the trigger sample is the first entry written after it, so the trigger
 *  lands exactly on that entry's first sample. */
function expandRLE(entries, post = 0) {
  let total = 0;
  for (const e of entries) total += (e >> 8) + 1;
  const samples = new Uint8Array(total), pre = entries.length - post;
  let at = 0, trigIdx = null;
  entries.forEach((e, k) => {
    if (post && k === pre) trigIdx = at;
    const len = (e >> 8) + 1;
    samples.fill(e & 0xff, at, at + len);
    at += len;
  });
  return { samples, trigIdx };
}

if (typeof module !== 'undefined') module.exports = { expandRLE };
