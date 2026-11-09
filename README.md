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
| AI rewrite with its own switch | The native AI rewrite step, which repairs grammar, false starts and punctuation, is one entry in a mode's transformation list. The Modes pane's Processing section has a dedicated switch for it beside the older OpenCode rewrite, so turning one on never turns the other off. |
| Remote-only rewrite selection | Select text, press a shortcut, speak an instruction, and the selection is replaced with the model's result. The selection is read through UI Automation, with a clipboard round trip when the app has no text pattern. It calls a remote endpoint only; the bundled small model did not execute a measured rewrite instruction. |
| Rewrite snippet match | An optional lab switch lets the remote model map a vague rewrite request of 3 to 24 words to one saved prompt snippet. An unknown reply keeps the instruction as spoken. |
| Vocabulary corrections and code-term wrap | The shared vocabulary turns `sounds_like` spellings into terms, and a mode's own vocabulary adds its corrections. Terms marked as code can be wrapped in backticks after the rewrite; the wrap is off by default. |
| Paste where the dictation started | On Windows the transcript returns to the window that was in front when the recording started, even after alt-tabbing during dictation. A closed or elevated window falls back to the current focus. |
| Shortcut guard | Named executables keep every Hex shortcut, and a separate switch suppresses the shortcuts in fullscreen apps. |
| Escape cancels | Escape discards a recording in both Hold and Toggle modes, and also cancels a rewrite recording, leaving no transcript and no paste. |
| Pane layout | General reads the hardware first, then the keys and their mode, then Vocabulary and Launch at login. Models puts each feature before the local server that serves it. |

## Getting started

The setup lives in two panes. General holds the microphone, the dictation
shortcut, the shortcut mode, Vocabulary, and launch at login. Models holds the
speech model, the dictation cleanup source, the rewrite selection endpoint, and
the local model server.

Add the AI rewrite step on the Modes pane: its switch adds the step to a mode's
transformations, and the list can be reordered so rules that read the raw words
run before the model and rules that shape its output run after.

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
