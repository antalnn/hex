//! Labs › Transcription endpoint's strings, kept beside
//! [`super::TRANSLATIONS`] and the French, Portuguese, and Russian tables so
//! those stay under the source cap.

/// Columns: Polish, Chinese (Simplified), Japanese, German, Spanish, Ukrainian.
#[rustfmt::skip]
pub(super) const TRANSLATIONS: &[(&str, [&str; 6])] = &[
    ("Transcription endpoint for other apps", ["Punkt końcowy transkrypcji dla innych aplikacji", "供其他应用使用的转写端点", "他のアプリ向けの文字起こしエンドポイント", "Transkriptions-Endpunkt für andere Apps", "Punto de conexión de transcripción para otras apps", "Кінцева точка транскрипції для інших застосунків"]),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", ["Pozwala aplikacjom na tym komputerze, takim jak OpenChamber, wysyłać dźwięk do Hex i odbierać gotowy tekst", "让本机上的应用（如 OpenChamber）把音频发给 Hex，并取回处理好的文字", "OpenChamber など、この PC 上のアプリが Hex に音声を送り、仕上がったテキストを受け取れるようにします", "Lässt Apps auf diesem PC, etwa OpenChamber, Audio an Hex senden und den fertigen Text zurückbekommen", "Permite que las apps de este PC, como OpenChamber, envíen audio a Hex y reciban el texto final", "Дає змогу застосункам на цьому ПК, як-от OpenChamber, надсилати аудіо до Hex і отримувати готовий текст"]),
];

#[rustfmt::skip]
pub(super) const FRENCH: &[(&str, &str)] = &[
    ("Transcription endpoint for other apps", "Point d'accès de transcription pour d'autres apps"),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", "Permet aux apps de ce PC, comme OpenChamber, d'envoyer de l'audio à Hex et de recevoir le texte final"),
];

#[rustfmt::skip]
pub(super) const PORTUGUESE: &[(&str, &str)] = &[
    ("Transcription endpoint for other apps", "Endpoint de transcrição para outros apps"),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", "Permite que apps neste PC, como o OpenChamber, enviem áudio ao Hex e recebam o texto final"),
];

#[rustfmt::skip]
pub(super) const RUSSIAN: &[(&str, &str)] = &[
    ("Transcription endpoint for other apps", "Конечная точка транскрипции для других приложений"),
    ("Lets apps on this PC, such as OpenChamber, send audio to Hex and get the finished text back", "Позволяет приложениям на этом ПК, например OpenChamber, отправлять звук в Hex и получать готовый текст"),
];
