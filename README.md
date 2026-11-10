# Hex (personal fork)

Hex is local voice dictation for Mac, Windows and Linux: press a shortcut,
transcribe on this machine, and paste the text where the caret is.

This repository is `antalnn/hex`, a personal fork of
[anomalyco/hex](https://github.com/anomalyco/hex). It is a quality-of-life
extension of the application: every change below is optional, and none is
required to dictate. The core transcription, models, meetings, and dictation
pipeline are the upstream app.

Fork work lives on `personal`. `dev` is a clean mirror of `upstream/dev` and is
reset, never merged into.

## What this fork adds

| Quality-of-life change | What it gives you |
|---|---|
| Local model server owned by the app | Hex starts one llama.cpp server when the local model is enabled and stops it with the process; on Windows the child runs in a job object, so a crash cannot leave it holding GPU memory. A server that already answers health checks is adopted instead of replaced. A missing program leaves the local model off while Remote keeps working. |
| AI cleanup, its own step | The native AI cleanup step repairs grammar, false starts and punctuation. It is its own entry in a mode's transformation list, separate from the older OpenCode rewrite, so turning one on never turns the other off. Output lists the steps that run. |
| Remote-only rewrite selection | Select text, press a shortcut, speak an instruction, and the selection is replaced with the model's result. The selection is read through UI Automation, with a clipboard round trip when the app has no text pattern. It calls a remote endpoint only; the bundled small model did not execute a measured rewrite instruction. |
| Rewrite semantic match | An optional lab switch lets the remote model match a spoken instruction to one of your saved snippets. An unknown reply keeps the instruction as spoken. |
| Vocabulary corrections and code-term wrap | The shared vocabulary turns `sounds_like` spellings into terms, and a mode's own vocabulary adds its corrections. Terms marked as code can be wrapped in backticks after the cleanup; the wrap is off by default. |
| Paste where the dictation started | On Windows the transcript returns to the window that was in front when the recording started, even after alt-tabbing during dictation. A closed or elevated window falls back to the current focus. |
| Shortcut guard | Named executables keep every Hex shortcut, and a separate switch suppresses the shortcuts in fullscreen apps. |
| Escape cancels | Escape discards a recording in both Hold and Toggle modes, and also cancels a rewrite recording, leaving no transcript and no paste. |
| Pane layout | General reads the hardware first, then the keys and their mode, then Vocabulary and Launch at login. Models puts each feature before the local server that serves it. |

## Getting started

The setup lives in two panes. General holds the microphone, the dictation
shortcut, the shortcut mode, Vocabulary, and launch at login. Models holds the
speech model, the dictation cleanup source, the rewrite selection endpoint, and
the local model server.

The AI cleanup step is one entry in a mode's transformation list, and the list
can be reordered so rules that read the raw words run before the model and
rules that shape its output run after. Output lists the steps that run; Edit
with agent changes them.

The upstream app installs with Homebrew (`brew install --cask anomalyco/tap/hex`),
the AUR (`yay -S hex-bin`), the Linux install script
(`curl -fsSL https://anoma.ly/hex/install-linux.sh | sh`), or Nix
(`nix profile add github:anomalyco/hex`).

## Local model

The local model is the dictation cleanup's model call, served by a llama.cpp
server Hex runs itself.

1. Open Models and find the Dictation card. Set its source to Local.
2. The local model card below shows the server state and Start or Stop. Hex
   starts the server with the app and stops it when Hex ends; on Windows the
   child runs in a job object, so a crash cannot leave it holding GPU memory.
3. Open the card's files sheet to see the llama.cpp folder and the model file.
   Add a model file there when the default is missing.

The settings those steps produce look like this:

```json
"llm": { "enabled": true, "program": null, "model": null, "port": 8081 }
```

With both paths unset, Hex resolves the server at
`<models>\local-llm\llama.cpp\llama-server.exe` and uses the default model in the
same `local-llm` folder, where `<models>` is `%LOCALAPPDATA%\hex\models` on
Windows. The resolver also honors `HEX_LLAMA_SERVER` and `llm.program`. A
missing program leaves the local model off and Remote keeps working for every
feature.

The local model is a text model, not a speech model: it receives the transcript
and returns a cleaned version. Hex loads any GGUF chat model its llama.cpp
server supports, such as a model from the Qwen family. Browse models on
[Hugging Face](https://huggingface.co/models), pick one that publishes GGUF
files, and add the file from the local model card's files sheet.

If another server already answers the port, Hex adopts it instead of starting a
second one. A server Hex recorded from an earlier crash is killed and replaced,
so it does not stay behind as an external process.

## Rewrites

Rewrite selection replaces text you already selected with a model's result.

1. General carries the Rewrite shortcut row. Record a key there.
2. Models carries the Rewrite selection card with the endpoint, the model, and
   the API key. This feature is remote only.
3. Select text, press the shortcut, and speak an instruction. The result
   replaces the selection in place.

```json
"rewrite": {
  "endpoint": "https://opencode.ai/inference/go/openai/v1/chat/completions",
  "model": "mimo-v2.6-flash",
  "api_key": "your key"
}
```

The card warns while the endpoint or model is empty. With no text selected the
HUD reports the failure and nothing pastes.

Labs carries a snippet match switch. When it is on, a rewrite instruction of 3
to 24 words may be mapped by the remote model to one saved prompt snippet. A
name the model invents, or a reply of none, leaves the instruction as spoken.

## Prompts

The Rewrite prompts sheet holds named snippets and the default prompt.

- A snippet is a name plus the text that replaces your instruction.
- The Rewrite selection card's sheet lists them in their saved order; the
  switch on the sheet decides whether a snippet is used when the instruction
  names one.
- `default_prompt` names the snippet an unnamed instruction starts from. Here
  the saved snippets are `prompt`, `polish`, `prompt concise`, `tighten`, and
  `structure`, and the default is `polish`.

The saved prompt text is private, so this row shows the shape of a snippet with
an invented prompt:

| Snippet | Prompt |
|---|---|
| `tighten` | `Keep every fact and remove filler. Keep my sentence order.` |

That prompt is an example, not a saved one. A snippet you save works the same
way: the model receives its text instead of your spoken words.

## How it all works

A dictation runs through the same stages on every OS, and each stage has one
place to change it:

1. Labs rules rewrite the raw transcript first: spoken formatting, spoken
   symbols, and filler words.
2. Vocabulary corrections replace `sounds_like` spellings with the saved term,
   from General › Vocabulary. A mode's own vocabulary and swaps apply next.
3. The mode's transformations run in their saved order, listed in Output. The
   native AI cleanup step runs where its id sits in that list; the personal
   rules in `~/.config/hex` follow their own order around it.
4. Paragraph and colon rules run after the model, and a saved snippet replaces a
   whole-utterance cue last, so its text is never reformatted by an earlier
   step.
5. The code-term wrap runs last, adding backticks around terms marked as code
   in Vocabulary. It is off by default.

Editing a loaded rule in `~/.config/hex` changes the trust hash, so Hex asks
for Allow in the Output pane once before the rules run again.

## Build and run

Requirements: Rust 1.97 or newer and the Vulkan SDK, with `VULKAN_SDK` set.

```powershell
cargo build --release
cargo fmt --all -- --check
cargo check --all-targets
cargo test
```

The first build compiles the speech model's Vulkan shaders and takes minutes.
Quit any running Hex before building or launching another copy: both installs
share `%APPDATA%\hex`, and the single-instance event allows one.

`hex start` opens the app, `hex quit` closes it, and
`hex window show --pane settings|models|modes|history|meetings` opens a pane
directly. Settings live in `%APPDATA%\hex\settings.json`; logs are in
`%LOCALAPPDATA%\hex\logs\process.log`.

## License

MIT, as upstream.
