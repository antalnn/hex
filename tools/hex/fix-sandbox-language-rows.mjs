// One-shot repair: the sandbox per-language tables take one string, not the
// six-column form the main table uses.
import fs from 'node:fs';

const shortKey = 'How well this works depends on how large the local model you installed is.';
const deadlineKey = 'Clean up deadline/timeout';

for (const file of [
  'ui-sandbox/src/desktop/i18n/french.rs',
  'ui-sandbox/src/desktop/i18n/portuguese.rs',
  'ui-sandbox/src/desktop/i18n/russian.rs',
]) {
  const raw = fs.readFileSync(file, 'utf8');
  const eol = raw.includes('\r\n') ? '\r\n' : '\n';
  const lines = raw.split(/\r?\n/);
  let repaired = 0;
  for (const key of [shortKey, deadlineKey]) {
    const at = lines.findIndex((line) => line.trimStart().startsWith(`(${JSON.stringify(key)}, [`));
    if (at < 0) continue;
    lines[at] = `    (${JSON.stringify(key)}, ${JSON.stringify(key)}),`;
    repaired += 1;
  }
  if (repaired !== 2) throw new Error(`${file}: repaired ${repaired} rows`);
  fs.writeFileSync(file, lines.join(eol));
  console.log(`repaired ${file}`);
}
