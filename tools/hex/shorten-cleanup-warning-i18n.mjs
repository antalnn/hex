// One-shot: shorten the AI cleanup model warning and name the deadline row
// in every interface table (app and sandbox).
import fs from 'node:fs';

const shortKey = 'How well this works depends on how large the local model you installed is.';
const deadlineKey = 'Clean up deadline/timeout';
const oldKey = 'Results depend on the local model you installed and how large it is.';

const warningTranslations = {
  'src/desktop/i18n.rs': [
    'Wynik zależy od rozmiaru zainstalowanego modelu lokalnego.',
    '效果取决于你安装的本地模型的大小。',
    '結果はインストールしたローカルモデルの大きさに左右されます。',
    'Das Ergebnis hängt von der Größe des installierten lokalen Modells ab.',
    'El resultado depende del tamaño del modelo local que instalaste.',
    'Результат залежить від розміру встановленої вами локальної моделі.',
  ],
  'src/desktop/i18n/french.rs': 'Le résultat dépend de la taille du modèle local que vous avez installé.',
  'src/desktop/i18n/portuguese.rs': 'O resultado depende do tamanho do modelo local que você instalou.',
  'src/desktop/i18n/russian.rs': 'Результат зависит от размера установленной вами локальной модели.',
};

const deadlineTranslations = {
  'src/desktop/i18n.rs': [
    'Limit czasu czyszczenia',
    '清理截止时间',
    'クリーンアップの期限',
    'Bereinigungsfrist',
    'Plazo de la limpieza',
    'Термін очищення',
  ],
  'src/desktop/i18n/french.rs': 'Délai de nettoyage',
  'src/desktop/i18n/portuguese.rs': 'Prazo da limpeza',
  'src/desktop/i18n/russian.rs': 'Срок очистки',
};

function quote(value) {
  return JSON.stringify(value);
}

function rustKey(key) {
  return JSON.stringify(key);
}

function warningLine(file) {
  const translation = warningTranslations[file];
  if (Array.isArray(translation)) {
    return `    (${rustKey(shortKey)}, [${translation.map(quote).join(', ')}]),`;
  }
  return `    (${rustKey(shortKey)}, ${quote(translation)}),`;
}

function deadlineLine(file) {
  const translation = deadlineTranslations[file];
  if (Array.isArray(translation)) {
    return `    (${rustKey(deadlineKey)}, [${translation.map(quote).join(', ')}]),`;
  }
  return `    (${rustKey(deadlineKey)}, ${quote(translation)}),`;
}

const sandbox = 'ui-sandbox/';
const files = [
  ...Object.keys(warningTranslations),
  'ui-sandbox/src/desktop/i18n.rs',
  'ui-sandbox/src/desktop/i18n/french.rs',
  'ui-sandbox/src/desktop/i18n/portuguese.rs',
  'ui-sandbox/src/desktop/i18n/russian.rs',
];

for (const file of files) {
  const raw = fs.readFileSync(file, 'utf8');
  const eol = raw.includes('\r\n') ? '\r\n' : '\n';
  const lines = raw.split(/\r?\n/);
  const isSandbox = file.startsWith(sandbox);
  const tableFile = isSandbox ? file.slice(sandbox.length) : file;
  const warning = isSandbox
    ? `    (${rustKey(shortKey)}, [${Array(6).fill(quote(shortKey)).join(', ')}]),`
    : warningLine(tableFile);
  const deadline = isSandbox
    ? `    (${rustKey(deadlineKey)}, [${Array(6).fill(quote(deadlineKey)).join(', ')}]),`
    : deadlineLine(tableFile);

  const warningAt = lines.findIndex((line) =>
    line.includes('Results depend on the local model you installed'),
  );
  if (warningAt < 0) throw new Error(`${file}: old warning row missing`);
  lines[warningAt] = warning;

  const deadlineAt = lines.findIndex((line) =>
    line.trimStart().startsWith('("Deadline",'),
  );
  if (deadlineAt < 0) throw new Error(`${file}: Deadline row missing`);
  if (lines.some((line) => line.trimStart().startsWith(`(${JSON.stringify(deadlineKey)}`))) {
    throw new Error(`${file}: deadline row already there`);
  }
  lines.splice(deadlineAt + 1, 0, deadline);

  const updated = lines.join(eol);
  if (!updated.includes(shortKey) || !updated.includes(deadlineKey)) {
    throw new Error(`${file}: the new rows did not land`);
  }
  fs.writeFileSync(file, updated);
  console.log(`updated ${file}`);
}
