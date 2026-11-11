# Hex (personal fork)

Fork of Hex (local voice dictation for Mac, Windows and Linux) that brings new
features, more customizations, and improves the set up and configuration
experience.

This repository is `antalnn/hex`, a personal fork of
[anomalyco/hex](https://github.com/anomalyco/hex). It tracks the latest
upstream `dev` and `main` updates and takes the upstream features that merge
cleanly, so it stays current while the fork work adds quality-of-life changes.
The core transcription, models, meetings and dictation pipeline are the
upstream app. Fork work lives on `personal`; `dev` is a clean mirror of
`upstream/dev` and is reset, never merged into. Every change below is
optional, and none is required to dictate.

## Contents

- [Added features](#added-features)
  - [Configuration and setup](#configuration-and-setup)
  - [AI cleanup, local and remote](#ai-cleanup-local-and-remote)
  - [Transformations: order and switches](#transformations-order-and-switches)
  - [Rewrite selection](#rewrite-selection)
  - [Application exclusion](#application-exclusion)
  - [Paste where the dictation started](#paste-where-the-dictation-started)
  - [Rewrite semantic match](#rewrite-semantic-match)
  - [Other changes](#other-changes)
- [Getting started](#getting-started)
- [Build and run](#build-and-run)
- [License](#license)

## Added features

### Configuration and setup

General reads the hardware first: Microphone, Keys, Shortcut mode and
Vocabulary, each a card of its own. Launch at login sits in Application.
Models puts every feature above the server that serves it, so the speech
model, the AI cleanup source and the rewrite endpoint each sit next to their
own settings.

### AI cleanup, local and remote

The AI cleanup step repairs grammar, false starts and punctuation. It is one
entry in a mode's transformation list, separate from the OpenCode rewrite, so
turning one on never turns the other off.

Its card carries Run on. Local serves the cleanup from the llama.cpp server Hex
runs itself. Remote sends it to an endpoint and model you name, with your API
key.

- Local: the Start with Hex switch above Start and Stop decides whether the
  server starts with the app. With that switch off, the server runs only when
  you press Start. On Windows the child runs in a job object, so a crash
  cannot leave it holding GPU memory. A server that already answers health
  checks is adopted instead of replaced.
- Remote: the endpoint, the model and the API key. A missing local program
  leaves the local model off while Remote keeps working for every feature.
- Clean up time limit takes 1 to 3600 seconds and aborts a call that outlives
  it. The text still pastes unchanged and the failure pill names the reason.
- The prompt sheet holds the cleanup prompt and reads "How well this works
  depends on how large the local model you installed is."

The local model is a chat model, not a speech model: it receives the transcript
and returns a cleaned version. Hex loads any GGUF model its llama.cpp server
supports, such as one from the Qwen family. Browse
[Hugging Face](https://huggingface.co/models) for a model that publishes GGUF
files.

### Transformations: order and switches

Output lists the steps a mode runs, in order. Drag a number to move its step.
Click a number without dragging to switch that step off. A switched-off step
keeps its row, dims, and is skipped by the pipeline and by Try. The AI cleanup
row has its own Edit button for its prompt.

The order decides what each step sees: rules that read the raw words run before
the model, and rules that shape its output run after. Editing a loaded rule in
`~/.config/hex` changes its trust hash, so Hex asks for Allow in Output once
before the rules run again.

### Rewrite selection

Select text, press the rewrite shortcut, and speak an instruction. The
selection is replaced with the model's result, so "make this a structured
prompt" rewrites the paragraph you highlighted.

- General carries the Rewrite shortcut row.
- Models carries the Rewrite selection card with the endpoint, the model and
  the API key. This feature is remote only: the bundled small model did not
  execute a measured rewrite instruction.
- The selection is read through UI Automation, with a clipboard round trip
  when the app has no text pattern.
- A rewrite recording stops at ten seconds.

This feature is not covered by [Paste where the dictation
started](#paste-where-the-dictation-started).

### Application exclusion

Named executables keep every Hex shortcut, so dictation keeps working in the
tools you list. A separate switch suppresses the shortcuts whenever a
full-screen app is in front.

### Paste where the dictation started

On Windows the transcript returns to the window that was in front when the
recording started, even after alt-tabbing during dictation. A closed or elevated
window falls back to the current focus.

This feature covers dictation only. It does not apply to rewrite selection, and
rewrite selection does not assure the prompt is pasted into the last selected
text the way a dictation paste does.

### Rewrite semantic match

Labs carries a switch that lets the model match an instruction to one of your
saved prompts by meaning instead of by name.

With the switch off, a full sentence may not match a prompt you named
`structure`. With it on, the model compares the instruction against your
configured prompts, picks the closest one, and applies that prompt to the
selected text. A reply that names nothing you saved leaves the instruction as
spoken.

### Other changes

- Escape cancels a recording in Hold and Toggle, and cancels a rewrite
  recording or a pending rewrite.
- Vocabulary corrections turn `sounds_like` spellings into the saved term, and
  a mode's own vocabulary adds its corrections.
- The code-term wrap puts backticks around terms marked as code, after the
  cleanup. It is off by default.

## Getting started

Install upstream Hex with Homebrew
(`brew install --cask anomalyco/tap/hex`), the AUR (`yay -S hex-bin`), the
Linux install script
(`curl -fsSL https://anoma.ly/hex/install-linux.sh | sh`), or Nix
(`nix profile add github:anomalyco/hex`). Then build this fork as described
below.

Open General first: pick the microphone, record the dictation and rewrite
shortcuts, and choose the shortcut mode. Then open Models and set the AI
cleanup source to Local or Remote. The mode's steps, their order and their
switches live in Output.

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

Settings live in `%APPDATA%\hex\settings.json`; logs are in
`%LOCALAPPDATA%\hex\logs\process.log`.

## License

MIT, as upstream.
