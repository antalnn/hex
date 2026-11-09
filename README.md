# Hex

Hex is local voice dictation for Mac, Windows and Linux.

## About this fork

This is a personal fork of [anomalyco/hex](https://github.com/anomalyco/hex),
tracked at [antalnn/hex](https://github.com/antalnn/hex). The core app is
unchanged: press a shortcut, transcribe on this machine, paste where the caret
is. Fork work lives on `personal`; `dev` mirrors `upstream/dev`.

### What's new

| Change | What it does |
|---|---|
| Local model server tied to the app | One llama.cpp server, ended with the process and held in a Windows job object. |
| Remote-only rewrite selection | The rewrite shortcut always calls a remote endpoint. |
| Rewrite snippet match lab | The model may map a vague rewrite request to a saved snippet. |
| Excluded apps | Named executables keep every Hex shortcut. |
| Escape cancels | Escape discards a recording, rewrite included. |
| Code-term wrap | Terms marked as code get backticks after the rewrite; off by default. |
| Paste where dictation started | The transcript returns to the window in front at the shortcut press (Windows). |
| Pane order | General reads hardware, keys, shortcut mode, Vocabulary, Launch at login; Models puts features before the server. |

### Dictation pipeline

1. Labs rules on the raw transcript: spoken formatting, spoken symbols, filler
   words.
2. Vocabulary corrections: each `sounds_like` spelling becomes its term, plus
   the default mode's spellings and `replacements`.
3. The older OpenCode rewrite, while a mode's Processing switch is on and an
   OpenCode service answers.
4. The mode's transformations in order. This fork runs `spoken-bullets`,
   `orphans`, `ai-rewrite`, `paragraphs`, `colon-cues`, `snippets`.
5. The code-term wrap, last.

The personal transformations run in QuickJS. Permission is a SHA-256 over every
loaded module, so any edit under `~/.config/hex` asks again; click Allow in the
Output pane after a batch.

### AI rewrite

AI rewrite is Hex's own cleanup step: a model repairs grammar, false starts and
punctuation, and an empty, too short or too changed reply is discarded. Add it
to a mode on the Modes pane, from Processing or the Transformations + menu,
then drag it to where the model should run. On Models › Dictation, choose Local
for the llama.cpp server Hex owns or Remote for an OpenAI-compatible endpoint.
The Processing switch's OpenCode transformation is the older rewrite, with its
own prompt, model and deadline, and needs a running OpenCode service.

### Remote model

Under Models › Dictation choose Remote for AI rewrite; the Rewrite selection
card has the fields for the rewrite shortcut. Both take an endpoint, a model
name and an optional API key (a bearer token). The endpoint may be a base
address, a `/v1` address or the full `/chat/completions` path; Hex appends the
rest. Rewrite selection stays remote only: the bundled small model did not
execute a measured rewrite instruction.

### Rewrite selection

Select text, press the rewrite shortcut, and speak an instruction. Hex reads
the selection through UI Automation, or a clipboard copy when the app has no
text pattern, calls the remote endpoint, and pastes the result over the
selection. A failed or empty rewrite pastes nothing and never enters History.
Snippets and the default prompt live in the Rewrite prompts sheet. Windows
only. Labs' Rewrite snippet match, off by default, lets the model map a vague
instruction of 3 to 24 words to a saved snippet; an unknown reply keeps the
words as spoken.

### Local model

Hex starts one llama.cpp server when the local model is enabled and ends it
with the process. A server Hex recorded and left behind is killed at the next
start; one it never started is adopted as external.

`llama-server` is found in this order: `HEX_LLAMA_SERVER`, `llm.program`,
`<models>/local-llm/llama.cpp/llama-server.exe`, then
`~/tools/llama-cpp/llama-server.exe`. The default model is
`<models>/local-llm/fluid-1-mini-q6_k.gguf` on port 8081. With no program the
local model is off, Remote keeps working, and the Models card says llama.cpp is
not installed.

### Shortcut guard and cancel

`shortcuts.excluded_processes` and `shortcuts.disable_in_fullscreen` stop every
Hex shortcut before it runs. Escape discards a recording in Hold and Toggle
alike, rewrite included, and leaves no transcript and no paste.

### Vocabulary

The shared vocabulary holds each term, its `sounds_like` spellings and a code
flag. The default mode's Replace words list is folded in once and hidden;
per-app modes keep their own Custom swaps. A term marked as code gets backticks
after the rewrite, leaving paths, code fences and longer identifiers alone.

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

Quit Hex first: both installs share `%APPDATA%\hex`, and the single-instance
event allows one. For UI work, point `HEX_APPLICATION_SUPPORT_DIR` at a folder
holding `settings.json`, `history.json` and `onboarding-complete`, with
`llm.model` set to the real GGUF.
