//! Labs > Transcription endpoint's strings, kept beside
//! [`super::TRANSLATIONS`] and the French, Portuguese, and Russian tables so
//! those stay under the source cap.

/// Columns: Polish, Chinese (Simplified), Japanese, German, Spanish, Ukrainian.
#[rustfmt::skip]
pub(super) const TRANSLATIONS: &[(&str, [&str; 6])] = &[
    ("Transcription endpoint for other apps", ["Punkt końcowy transkrypcji dla innych aplikacji", "供其他应用使用的转写端点", "他のアプリ向けの文字起こしエンドポイント", "Transkriptions-Endpunkt für andere Apps", "Punto de conexión de transcripción para otras apps", "Кінцева точка транскрипції для інших застосунків"]),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", ["Pozwala aplikacjom na tym komputerze, takim jak OpenChamber, wysyłać dźwięk do Hex i odbierać gotowy tekst", "让本机上的应用（如 OpenChamber）把音频发给 Hex，并取回处理好的文字", "OpenChamber など、この PC 上のアプリが Hex に音声を送り、仕上がったテキストを受け取れるようにします", "Lässt Apps auf diesem PC, etwa OpenChamber, Audio an Hex senden und den fertigen Text zurückbekommen", "Permite que las apps de este PC, como OpenChamber, envíen audio a Hex y reciban el texto final", "Дає змогу застосункам на цьому ПК, як-от OpenChamber, надсилати аудіо до Hex і отримувати готовий текст"]),
    ("Port {} is in use by another program", ["Port {} jest zajęty przez inny program", "端口 {} 已被其他程序占用", "ポート {} は別のプログラムが使用中です", "Port {} wird von einem anderen Programm verwendet", "El puerto {} está en uso por otro programa", "Порт {} зайнятий іншою програмою"]),
    ("Could not start", ["Nie udało się uruchomić", "无法启动", "起動できませんでした", "Konnte nicht gestartet werden", "No se pudo iniciar", "Не вдалося запустити"]),
    ("Server URL", ["Adres serwera", "服务器 URL", "サーバー URL", "Server-URL", "URL del servidor", "URL сервера"]),
    ("Endpoint path", ["Ścieżka punktu końcowego", "端点路径", "エンドポイントのパス", "Endpunkt-Pfad", "Ruta del endpoint", "Шлях кінцевої точки"]),
    ("Use any model name; Hex uses the model selected in General.", ["Użyj dowolnej nazwy modelu; Hex używa modelu wybranego w sekcji Ogólne.", "模型名称随意填写；Hex 使用“通用”中选定的模型。", "モデル名は何でも構いません。Hex は「一般」で選択したモデルを使います。", "Beliebiger Modellname; Hex verwendet das unter Allgemein gewählte Modell.", "Usa cualquier nombre de modelo; Hex usa el modelo elegido en General.", "Вкажіть будь-яку назву моделі; Hex використовує модель, вибрану в розділі Загальні."]),
];

#[rustfmt::skip]
pub(super) const FRENCH: &[(&str, &str)] = &[
    ("Transcription endpoint for other apps", "Point d'accès de transcription pour d'autres apps"),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", "Permet aux apps de ce PC, comme OpenChamber, d'envoyer de l'audio à Hex et de recevoir le texte final"),
    ("Port {} is in use by another program", "Le port {} est utilisé par un autre programme"),
    ("Could not start", "Démarrage impossible"),
    ("Server URL", "URL du serveur"),
    ("Endpoint path", "Chemin du point d'accès"),
    ("Use any model name; Hex uses the model selected in General.", "Utilisez n'importe quel nom de modèle ; Hex utilise le modèle choisi dans Général."),
];

#[rustfmt::skip]
pub(super) const PORTUGUESE: &[(&str, &str)] = &[
    ("Transcription endpoint for other apps", "Endpoint de transcrição para outros apps"),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", "Permite que apps neste PC, como o OpenChamber, enviem áudio ao Hex e recebam o texto final"),
    ("Port {} is in use by another program", "A porta {} está em uso por outro programa"),
    ("Could not start", "Não foi possível iniciar"),
    ("Server URL", "URL do servidor"),
    ("Endpoint path", "Caminho do endpoint"),
    ("Use any model name; Hex uses the model selected in General.", "Use qualquer nome de modelo; o Hex usa o modelo escolhido em Geral."),
];

#[rustfmt::skip]
pub(super) const RUSSIAN: &[(&str, &str)] = &[
    ("Transcription endpoint for other apps", "Конечная точка транскрипции для других приложений"),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", "Позволяет приложениям на этом ПК, например OpenChamber, отправлять звук в Hex и получать готовый текст"),
    ("Port {} is in use by another program", "Порт {} занят другой программой"),
    ("Could not start", "Не удалось запустить"),
    ("Server URL", "URL сервера"),
    ("Endpoint path", "Путь конечной точки"),
    ("Use any model name; Hex uses the model selected in General.", "Укажите любое имя модели; Hex использует модель, выбранную в разделе «Основные»."),
];
