// One-shot: add the AI cleanup setup window strings to the app tables and
// drop the where-it-runs lines the new card no longer shows. The sandbox
// tables already carry the new keys in English.
import fs from 'node:fs';

const main = 'src/desktop/i18n.rs';
const languages = {
  'src/desktop/i18n/french.rs': {
    'AI cleanup setup': 'Configuration du nettoyage IA',
    'Where cleanup runs and how long it may wait.':
      "Où s'exécute le nettoyage et combien de temps il peut attendre.",
    'Cleanup setup': 'Configuration du nettoyage',
    '{} seconds': '{} secondes',
    'Local · {} · server running': 'Local · {} · serveur en marche',
    'Local · {} · server off': 'Local · {} · serveur arrêté',
    'Remote · {}': 'Distant · {}',
  },
  'src/desktop/i18n/portuguese.rs': {
    'AI cleanup setup': 'Configuração da limpeza com IA',
    'Where cleanup runs and how long it may wait.':
      'Onde a limpeza é executada e quanto tempo ela pode esperar.',
    'Cleanup setup': 'Configuração da limpeza',
    '{} seconds': '{} segundos',
    'Local · {} · server running': 'Local · {} · servidor em execução',
    'Local · {} · server off': 'Local · {} · servidor desligado',
    'Remote · {}': 'Remoto · {}',
  },
  'src/desktop/i18n/russian.rs': {
    'AI cleanup setup': 'Настройка ИИ-очистки',
    'Where cleanup runs and how long it may wait.':
      'Где работает очистка и сколько она может ждать.',
    'Cleanup setup': 'Настройка очистки',
    '{} seconds': '{} секунд',
    'Local · {} · server running': 'Локально · {} · сервер работает',
    'Local · {} · server off': 'Локально · {} · сервер выключен',
    'Remote · {}': 'Удалённо · {}',
  },
};

const mainTranslations = {
  'AI cleanup setup': [
    'Konfiguracja czyszczenia AI',
    'AI 清理设置',
    'AI クリーンアップの設定',
    'KI-Bereinigung einrichten',
    'Configuración de la limpieza con IA',
    'Налаштування ІІ-очищення',
  ],
  'Where cleanup runs and how long it may wait.': [
    'Gdzie działa czyszczenie i jak długo może czekać.',
    '清理在哪里运行，以及最多等待多久。',
    'クリーンアップの実行先と待ち時間の上限。',
    'Wo die Bereinigung läuft und wie lange sie warten darf.',
    'Dónde se ejecuta la limpieza y cuánto puede esperar.',
    'Де працює очищення і скільки воно може чекати.',
  ],
  'Cleanup setup': [
    'Konfiguracja czyszczenia',
    '清理设置',
    'クリーンアップの設定',
    'Bereinigung einrichten',
    'Configuración de la limpieza',
    'Налаштування очищення',
  ],
  '{} seconds': ['{} sekund', '{} 秒', '{} 秒', '{} Sekunden', '{} segundos', '{} секунд'],
  'Local · {} · server running': [
    'Lokalnie · {} · serwer działa',
    '本地 · {} · 服务器运行中',
    'ローカル · {} · サーバー稼働中',
    'Lokal · {} · Server läuft',
    'Local · {} · servidor en marcha',
    'Локально · {} · сервер працює',
  ],
  'Local · {} · server off': [
    'Lokalnie · {} · serwer wyłączony',
    '本地 · {} · 服务器已关闭',
    'ローカル · {} · サーバー停止中',
    'Lokal · {} · Server aus',
    'Local · {} · servidor apagado',
    'Локально · {} · сервер вимкнено',
  ],
  'Remote · {}': [
    'Zdalnie · {}',
    '远程 · {}',
    'リモート · {}',
    'Remote · {}',
    'Remoto · {}',
    'Віддалено · {}',
  ],
};

const dead = [
  'Off: each dictation is pasted as transcribed, with no model step.',
  'Runs on the local model server when it is running.',
  'Runs on the local model server.',
  'Runs on the remote endpoint.',
  'The model server AI cleanup uses when it runs on Local. Its controls sit with that choice above.',
];

function isRow(line, key) {
  return line.trimStart().startsWith(`(${JSON.stringify(key)},`);
}

for (const file of [main, ...Object.keys(languages)]) {
  const raw = fs.readFileSync(file, 'utf8');
  const eol = raw.includes('\r\n') ? '\r\n' : '\n';
  const lines = raw.split(/\r?\n/);
  const anchor = lines.findIndex((line) => isRow(line, 'Clean up time limit'));
  if (anchor < 0) throw new Error(`${file}: the time limit row is missing`);
  const isMain = file === main;
  const added = [];
  for (const key of Object.keys(isMain ? mainTranslations : languages[file])) {
    if (lines.some((line) => isRow(line, key))) continue;
    added.push(
      isMain
        ? `    (${JSON.stringify(key)}, [${mainTranslations[key].map((value) => JSON.stringify(value)).join(', ')}]),`
        : `    (${JSON.stringify(key)}, ${JSON.stringify(languages[file][key])}),`,
    );
  }
  lines.splice(anchor + 1, 0, ...added);
  const kept = lines.filter((line) => !dead.some((key) => isRow(line, key)));
  fs.writeFileSync(file, kept.join(eol));
  console.log(`updated ${file}: +${added.length}, -${lines.length - kept.length}`);
}
