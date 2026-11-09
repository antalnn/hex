# Hex

Hex is local voice dictation for Mac, Windows and Linux.

## About this fork

This is a personal fork of [anomalyco/hex](https://github.com/anomalyco/hex),
tracked at [antalnn/hex](https://github.com/antalnn/hex). The core app is
unchanged: press a shortcut, transcribe on this machine, and paste the text
where the caret is. The fork adds the changes below and keeps them on the
`personal` branch; `dev` is a clean mirror of `upstream/dev`.

### Dictation pipeline

The native pipeline runs in this order:

1. Labs rules on the raw transcript: spoken formatting, spoken symbols, filler
   words.
2. Vocabulary corrections: every `sounds_like` spelling in the shared list
   becomes its term, plus the default mode's own spellings and `replacements`.
3. Personal transformations from `~/.config/hex`, in the mode's order:
   `spoken-bullets`, `orphans`, `local-rewrite`, `paragraphs`, `colon-cues`,
   `snippets`.
   - `spoken-bullets` builds dash and numbered lists from spoken ordinals and
     word numbers.
   - `orphans` deletes an abandoned sentence such as "I wanted to." when it is
     not the last sentence.
   - `local-rewrite` repairs grammar, false starts and punctuation through the
     local llama.cpp server on `127.0.0.1:8081`.
   - `paragraphs` breaks a long block at a sentence boundary, aiming for a
     break every 70 words inside its 90-word floor.
   - `colon-cues` ends a dictation on a colon before "the following ..." or
     "as follows".
   - `snippets` replaces a whole spoken cue with saved text.
4. The code-term wrap marks vocabulary terms flagged as code with backticks.

The transformations run in QuickJS. The first run asks for permission, and the
approval is a SHA-256 over every loaded module: editing any file in
`~/.config/hex` asks again. Click Allow in the Output pane after each batch of
edits.

### Rewrite selection

Select text, press the rewrite shortcut, and speak an instruction. Hex reads
the selection through UI Automation, or by a clipboard copy when the app has no
text pattern, calls a remote OpenAI-compatible endpoint, and pastes the result
over the selection. A failed or empty rewrite pastes nothing and never enters
History.

Rewrite selection is remote only. The bundled 1.9B model did not execute a
rewrite instruction under any of the nine instruction and five framing variants
that were measured (2026-11-08), while the remote model did. The endpoint,
model and key live on the Models pane's Rewrite selection card; snippets and
the default prompt live in its Rewrite prompts sheet. Windows only.

Labs has a Rewrite snippet match switch, off by default: when it is on, the
remote model may match a vague spoken instruction to one saved snippet. It only
runs for instructions of 3 to 24 words that open with a rewrite request, and an
unknown reply keeps the instruction as spoken.

### Models and settings

The Models pane puts the features first: Dictation (speech model, language, and
a Local or Remote cleanup) and Rewrite selection (remote endpoint, model, key),
then Local model, the server that serves dictation cleanup when Dictation is
set to Local. General reads the microphone first, then the dictation key and
the rewrite key, the shortcut mode and its cancel key, Vocabulary, and Launch
at login. Rows most people never change sit behind Advanced, where Excluded
apps opens Dictation and Paste where dictation started sits under Paste last
dictation.

### Local model

Hex starts one llama.cpp server when the local model is enabled and ends it
when Hex ends. The child runs inside a Windows job object, so a crash or a
force kill cannot leave it holding GPU memory. A server Hex recorded and left
behind is killed at the next start; a server Hex never started is adopted as
external and left alone.

`llama-server` is found in this order: `HEX_LLAMA_SERVER`, `llm.program`,
`<models>/local-llm/llama.cpp/llama-server.exe`, then
`~/tools/llama-cpp/llama-server.exe`. The default model is
`<models>/local-llm/fluid-1-mini-q6_k.gguf` on port 8081. With no program, the
local model is off and Remote keeps working; the Models card says llama.cpp is
not installed.

### Shortcut guard and cancel

`shortcuts.excluded_processes` and `shortcuts.disable_in_fullscreen` stop every
Hex shortcut before it runs, so a named game keeps its keys. Escape discards a
recording in both Hold and Toggle modes, including a rewrite recording, and
leaves no transcript and no paste.

### Vocabulary

The shared vocabulary is the source of the corrections: each entry holds a
term, its `sounds_like` spellings and a code flag. The default mode's Replace
words list is folded into it once and hidden; per-app modes keep their own
Custom swaps. Marking a term as code lets the post-rewrite pass wrap it in
backticks, leaving paths, code fences and longer identifiers alone. The wrap is
off by default.

### Other fork behavior

- Paste where dictation started returns to the window that was in front when
  the shortcut went down, even after alt-tabbing mid-dictation (Windows).
- The local API and the `hex` CLI are unchanged.
- The installed build at `%APPDATA%\hex\versions\3.3.0\hex.exe` is this fork's
  release build. `HEX_DISABLE_UPDATES=1` is set in the user environment so an
  update check cannot replace it.

### Branches and building

- `personal` holds the fork's commits on top of `upstream/dev`.
- `dev` is a clean mirror of `upstream/dev`; reset it, never merge into it.

Update `personal` with `git checkout personal && git merge upstream/dev`, and
`dev` with `git fetch upstream && git reset --hard upstream/dev`.

The build needs Rust 1.97 or newer and the Vulkan SDK (`VULKAN_SDK`). From the
fork root:

```powershell
cargo fmt --all -- --check
cargo check --all-targets
cargo test
cargo build --release
```

Quit the running Hex before building or launching the other build: both
installs share `%APPDATA%\hex`, and the single-instance event allows one of them.
Test UI changes against an isolated data root by setting
`HEX_APPLICATION_SUPPORT_DIR` to a folder holding a copy of `settings.json`,
`history.json` and `onboarding-complete`. Set `llm.model` in that copy to the
real GGUF, because the models folder resolves under the isolated root.
