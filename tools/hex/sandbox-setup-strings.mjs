// One-shot: add the AI cleanup setup window strings to the sandbox tables.
import fs from 'node:fs';

const keys = [
  'AI cleanup setup',
  'Where cleanup runs and how long it may wait.',
  'Cleanup setup',
  '{} seconds',
  'Local · {} · server running',
  'Local · {} · server off',
  'Remote · {}',
];

const files = [
  'ui-sandbox/src/desktop/i18n.rs',
  'ui-sandbox/src/desktop/i18n/french.rs',
  'ui-sandbox/src/desktop/i18n/portuguese.rs',
  'ui-sandbox/src/desktop/i18n/russian.rs',
];

for (const file of files) {
  const raw = fs.readFileSync(file, 'utf8');
  const eol = raw.includes('\r\n') ? '\r\n' : '\n';
  const lines = raw.split(/\r?\n/);
  const main = file.endsWith('desktop/i18n.rs');
  const anchor = lines.findIndex((line) =>
    line.trimStart().startsWith('("Clean up time limit",'),
  );
  if (anchor < 0) throw new Error(`${file}: the time limit row is missing`);
  const added = keys.map((key) =>
    main
      ? `    (${JSON.stringify(key)}, [${Array(6).fill(JSON.stringify(key)).join(', ')}]),`
      : `    (${JSON.stringify(key)}, ${JSON.stringify(key)}),`,
  );
  const missing = keys.filter(
    (key) => !lines.some((line) => line.trimStart().startsWith(`(${JSON.stringify(key)},`)),
  );
  if (missing.length !== keys.length) {
    throw new Error(`${file}: some keys already exist: ${missing.join(' | ')}`);
  }
  lines.splice(anchor + 1, 0, ...added);
  fs.writeFileSync(file, lines.join(eol));
  console.log(`updated ${file}`);
}
