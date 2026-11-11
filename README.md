# Hex (personal fork)

This repository is a fork of
[anomalyco/hex](https://github.com/anomalyco/hex). It tracks the latest
upstream `dev` and `main` updates and takes the upstream features that merge
cleanly, so it stays up-to-date with the official release while the fork contains
quality-of-life changes. The core transcription, models, meetings and dictation pipeline are the
upstream app.

## Contents

- [Added features](#added-features)
  - [AI cleanup, local and remote](#ai-cleanup-local-and-remote)
  - [Output transformations](#output-transformations)
  - [Rewrite selection](#rewrite-selection)
  - [Application exclusion](#application-exclusion)
  - [Paste where the dictation started](#paste-where-the-dictation-started)
  - [Rewrite semantic match](#rewrite-semantic-match)
  - [Other changes](#other-changes)
- [Getting started](#getting-started)
- [Build and run](#build-and-run)
- [License](#license)

## Added features

### AI cleanup, local and remote

AI cleanup (previously dictation's "post-processing") uses a prompt that you configure
on the dictated text.

Configure where the model will run:

- Local: start a local llama.cpp server and load a model through
  GGUF files
- Remote: use a remote model by configuring the endpoint, the model and the API key

Find local models on [Hugging Face](https://huggingface.co/models), search for GGUF files.

### Output transformations

This fork makes the numbering for transformation in the `Output` tab draggable and clickable:

- Drag to reorder transformations
- Click to enable/disable

The upstream application only allows this configuration through `settings.json`, which is inconvenient
when you just want to change the execution order or toggle them directly inside of the interface.

### Rewrite selection

Rewrite instructions are customizable in the `Models` section.
Name an instruction, assign it a prompt, then run it by speaking the keyword or snippet name;
you can also use `Rewrite semantic match` to let the model match a spoken instruction to the snippet.

To use one, select text, press the rewrite shortcut, and speak the instruction.
If your spoken instruction contains both the keyword and the snippet name, it will invoke the instruction.

The selection is replaced with the model's result, so "make this a structured
prompt" rewrites the paragraph you highlighted.

This feature is remote only and Windows only: small models cannot execute a
measured rewrite instruction, and the rewrite shortcut is not bound on macOS or
Linux.

This feature is not covered by [Paste where the dictation
started](#paste-where-the-dictation-started).

### Application exclusion

Go to General->Excluded apps to disable all Hex shortcuts for the specified
programs. Contains fullscreen detection - toggle to automatically block
shortcuts when any window is in fullscreen exclusive.

### Paste where the dictation started

Enable this setting to automatically paste the transcript to the window
or text box where dictation started. This lets you Alt-Tab anywhere and speak freely;
when you stop dictating, the text pastes automatically into that window.
It should work in most programs, though some applications may not support it.

This is a Windows-only feature and it only applies to dictation; rewrite selection
is not safeguarded through it.

### Rewrite semantic match

Rewrite semantic match lets the model match a spoken instruction to one of
your saved prompts by meaning instead of by name.

With it off, a full sentence may not match a prompt named `structure`.

With it on, the closest prompt applies; unmatched instructions run as spoken.

### Other changes

- Escape cancels a recording in Hold and Toggle, and cancels a rewrite
  recording or a pending rewrite.
- Turned "sounds like" into a unified Vocabulary that transforms no matter the model.

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
