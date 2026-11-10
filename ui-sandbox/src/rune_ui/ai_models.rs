//! The Models pane: where each model call goes.
//!
//! Three cards and nothing else, so the pane stays a switchboard rather than a
//! second General. The features come first: Dictation picks `Local` or
//! `Remote`, and the native AI cleanup step runs on whichever it names;
//! Rewrite selection always calls a remote endpoint, so its card shows the
//! address, model and key with no Local choice. The server that serves the
//! Local choice, Local model, comes last: it is the llama.cpp server Hex owns,
//! shown here instead of in the sidebar and Output. Remote is any
//! OpenAI-compatible endpoint, a hosted provider or a server on the network,
//! with its own model name and an optional bearer token.
//!
//! The Rewrite prompts sheet holds Rewrite selection's snippets and its default
//! prompt, edited like the Vocabulary sheet and saved when typing pauses.

use rune::{El, TextEdit, col, row, text};
use rune_kit::{ControlSize, TextArea, TextInput};

use super::controls::{
    ButtonKind, DESCRIPTION, METRICS, Paint, button, described_row, note, page, segmented,
    setting_row, toggle_row,
};
use super::lists::remove_button;
use super::{Request, Ui};
use crate::app_settings::{AiSource, AppSettings, RemoteModel};
use crate::desktop::host::DesktopAction;
use crate::desktop::i18n::{tr, tr_fill};

const FIELD_WIDTH: f32 = 280.0;
/// One snippet row: its two fields.
const ROW_HEIGHT: f32 = 36.0;
const PADDING: f32 = 18.0;
const MODAL_WIDTH: f32 = 640.0;
/// What the local server serves and the switch that turns it on, said on its
/// card. The card above it carries the AI cleanup switch and prompt.
const LOCAL_MODEL_PURPOSE: &str = "The model server the AI cleanup card uses when Dictation is set to Local. Hex starts it with the app.";

/// The endpoint, model and key of one feature's remote call.
#[derive(Default)]
struct SourceFields {
    endpoint: TextEdit,
    model: TextEdit,
    key: TextEdit,
}

impl SourceFields {
    fn new(remote: &RemoteModel) -> Self {
        Self {
            endpoint: TextEdit::new(remote.endpoint.clone()),
            model: TextEdit::new(remote.model.clone()),
            key: TextEdit::new(remote.api_key.clone()),
        }
    }

    /// Writes the typed values into `remote`.
    fn project(&self, remote: &mut RemoteModel) {
        remote.endpoint = self.endpoint.text.trim().to_string();
        remote.model = self.model.text.trim().to_string();
        remote.api_key = self.key.text.trim().to_string();
    }
}

/// One named instruction Rewrite selection can be told to use.
struct SnippetRow {
    id: u64,
    name: TextEdit,
    prompt: TextEdit,
}

/// The pane's typed state: both features' remote fields, and the Rewrite
/// prompts sheet's contents.
#[derive(Default)]
pub(super) struct ModelSources {
    dictation: SourceFields,
    rewrite: SourceFields,
    default_prompt: TextEdit,
    snippets: Vec<SnippetRow>,
    next_id: u64,
    /// What an edit for a removed snippet lands in; never saved.
    detached: TextEdit,
    pub(super) prompts_open: bool,
    /// The AI cleanup prompt sheet and its text.
    cleanup_prompt: TextEdit,
    pub(super) cleanup_open: bool,
}

impl ModelSources {
    pub(super) fn new(settings: &AppSettings) -> Self {
        let mut editor = Self::default();
        editor.reload(settings);
        editor
    }

    /// Rebuilds the fields from `settings`.
    pub(super) fn reload(&mut self, settings: &AppSettings) {
        self.dictation = SourceFields::new(&settings.dictation_rewrite.remote);
        self.rewrite = SourceFields::new(&settings.rewrite.remote);
        self.default_prompt = TextEdit::new(settings.rewrite.default_prompt.clone());
        self.cleanup_prompt = TextEdit::new(settings.ai_cleanup.prompt.clone());
        self.snippets = settings
            .rewrite
            .snippets
            .iter()
            .map(|snippet| {
                let id = self.take_id();
                SnippetRow {
                    id,
                    name: TextEdit::new(snippet.name.clone()),
                    prompt: TextEdit::new(snippet.prompt.clone()),
                }
            })
            .collect();
    }

    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Writes the typed fields into `settings`: the remote addresses, the
    /// default prompt, and the snippets, dropping blank ones.
    pub(super) fn project(&self, settings: &mut AppSettings) {
        self.dictation
            .project(&mut settings.dictation_rewrite.remote);
        self.rewrite.project(&mut settings.rewrite.remote);
        settings.rewrite.default_prompt = self.default_prompt.text.trim().to_string();
        settings.ai_cleanup.prompt = self.cleanup_prompt.text.trim().to_string();
        settings.rewrite.snippets = self
            .snippets
            .iter()
            .filter_map(|snippet| {
                let name = snippet.name.text.trim();
                let prompt = snippet.prompt.text.trim();
                (!name.is_empty() && !prompt.is_empty()).then(|| {
                    crate::app_settings::RewriteSnippet {
                        name: name.to_string(),
                        prompt: prompt.to_string(),
                    }
                })
            })
            .collect();
    }

    fn field(&mut self, id: u64, prompt: bool) -> &mut TextEdit {
        match self.snippets.iter_mut().find(|snippet| snippet.id == id) {
            Some(snippet) if prompt => &mut snippet.prompt,
            Some(snippet) => &mut snippet.name,
            None => {
                self.detached = TextEdit::default();
                &mut self.detached
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Open and close

/// Opens the Rewrite prompts sheet, with one empty row when there are none.
pub(super) fn open_prompts(ui: &mut Ui) {
    super::settings::cancel_capture(ui);
    super::picker::close(ui);
    super::explainer::close(ui);
    super::sound_sets::close(ui);
    ui.model_sources.prompts_open = true;
    ui.model_sources.cleanup_open = false;
    if ui.model_sources.snippets.is_empty() {
        add_snippet(ui);
    }
}

/// Saves what was typed and closes the sheet.
pub(super) fn close_prompts(ui: &mut Ui) {
    if !ui.model_sources.prompts_open {
        return;
    }
    ui.model_sources.prompts_open = false;
    ui.flush_drafts();
}

/// Opens the AI cleanup prompt sheet, closing the other Models sheet.
pub(super) fn open_cleanup_prompt(ui: &mut Ui) {
    super::settings::cancel_capture(ui);
    super::picker::close(ui);
    super::explainer::close(ui);
    super::sound_sets::close(ui);
    ui.model_sources.prompts_open = false;
    ui.model_sources.cleanup_open = true;
}

/// Saves what was typed and closes the AI cleanup prompt sheet.
pub(super) fn close_cleanup_prompt(ui: &mut Ui) {
    if !ui.model_sources.cleanup_open {
        return;
    }
    ui.model_sources.cleanup_open = false;
    ui.flush_drafts();
}

fn add_snippet(ui: &mut Ui) -> u64 {
    for snippet in &mut ui.model_sources.snippets {
        snippet.name.focused = false;
        snippet.prompt.focused = false;
    }
    let mut name = TextEdit::default();
    name.focused = true;
    let id = ui.model_sources.take_id();
    ui.model_sources.snippets.push(SnippetRow {
        id,
        name,
        prompt: TextEdit::default(),
    });
    id
}

fn add_prompt(ui: &mut Ui) {
    add_snippet(ui);
}

fn remove_snippet(ui: &mut Ui, id: u64) {
    if let Some(index) = ui
        .model_sources
        .snippets
        .iter()
        .position(|snippet| snippet.id == id)
    {
        ui.model_sources.snippets.remove(index);
        ui.flush_drafts();
    }
}

// ---------------------------------------------------------------------------
// The pane

pub(super) fn pane(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> (Vec<El>, El) {
    let content = page("content")
        .child(dictation_card(ui, p, layer))
        .child(rewrite_card(ui, p))
        .child(ai_cleanup_card(ui, p))
        .child(local_model_card(ui, p));
    (
        Vec::new(),
        super::scroll_body(p.k, "models-scroll", content),
    )
}

/// AI cleanup's switch, the prompt it follows, and where it runs. The card
/// sits directly above the local model card, so the switch, the prompt and
/// the server that serves the Local choice read together.
fn ai_cleanup_card(ui: &Ui, p: Paint) -> El {
    let enabled = ui.settings.ai_cleanup.enabled;
    let local = ui.settings.dictation_rewrite.source == AiSource::Local;
    let running = ui
        .view
        .as_ref()
        .and_then(|view| view.local_model.as_ref())
        .is_some_and(|snapshot| {
            matches!(
                snapshot.state,
                crate::local_model::State::Ready | crate::local_model::State::External
            )
        });
    let where_it_runs = if !enabled {
        tr("Off: each dictation is pasted as transcribed, with no model step.")
    } else if local && running {
        tr("Runs on the local model server.")
    } else if local {
        tr("Runs on the local model server when it is running.")
    } else {
        tr("Runs on the remote endpoint Dictation names.")
    };
    let rows = vec![
        toggle_row(
            p,
            "ai-cleanup-enabled",
            tr("AI cleanup"),
            Some(tr("Fix grammar, punctuation and paragraphs with a model")),
            enabled,
            |ui: &mut Ui| {
                ui.update(|settings| settings.ai_cleanup.enabled = !settings.ai_cleanup.enabled);
            },
        ),
        setting_row(
            p,
            "ai-cleanup-prompt",
            tr("Prompt"),
            Some(tr("The instruction the model follows")),
            button(
                p,
                "open-ai-cleanup-prompt",
                tr("Edit"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| open_cleanup_prompt(ui),
            ),
        ),
        note("ai-cleanup-runs", where_it_runs, p.muted()),
    ];
    super::controls::section(p, "ai-cleanup-section", tr("AI cleanup"), rows)
}

/// The llama.cpp server Hex owns, with its state and a start or stop button.
fn local_model_card(ui: &Ui, p: Paint) -> El {
    let snapshot = ui.view.as_ref().and_then(|view| view.local_model.as_ref());
    let (title, start) = local_model_copy(snapshot.map(|snapshot| snapshot.state));
    let detail = format!("127.0.0.1:{}", ui.settings.llm.port);
    let control = button(
        p,
        if start {
            "local-model-start"
        } else {
            "local-model-stop"
        },
        if start {
            tr("Start server")
        } else {
            tr("Stop server")
        },
        ButtonKind::Secondary,
        move |ui: &mut Ui, _| {
            ui.send(Request::Desktop(if start {
                DesktopAction::StartLocalModel
            } else {
                DesktopAction::StopLocalModel
            }));
        },
    );
    super::controls::section(
        p,
        "local-model-section",
        tr("Local model"),
        vec![
            toggle_row(
                p,
                "local-model-auto-start",
                tr("Start with Hex"),
                Some(tr(
                    "Start the local model server when Hex opens, and stop it when Hex quits",
                )),
                ui.settings.llm.enabled,
                |ui: &mut Ui| {
                    ui.update(|settings| settings.llm.enabled = !settings.llm.enabled);
                },
            ),
            described_row(
                p,
                "local-model-state",
                &title,
                Some((detail, p.muted())),
                control,
            ),
            note("local-model-purpose", tr(LOCAL_MODEL_PURPOSE), p.muted()),
            described_row(
                p,
                "local-model-files",
                tr("Local model files"),
                Some((
                    tr("llama.cpp and the GGUF the server loads").to_string(),
                    p.muted(),
                )),
                super::controls::value_button(
                    p,
                    "local-model-files-control",
                    tr("Local model files"),
                    tr("Edit"),
                    &rune_theme_opencode::icons::CHEVRON_RIGHT,
                    super::local_models::open,
                ),
            ),
        ],
    )
}

/// The card's title and whether it offers Start, for each server state.
fn local_model_copy(state: Option<crate::local_model::State>) -> (&'static str, bool) {
    match state {
        Some(crate::local_model::State::Ready) | Some(crate::local_model::State::External) => {
            (tr("Local model server running"), false)
        }
        Some(crate::local_model::State::Starting) => (tr("Local model server starting"), false),
        Some(crate::local_model::State::Failed) => (tr("Local model server failed"), true),
        Some(crate::local_model::State::Missing) => (
            tr("llama.cpp is not installed, so the local model is off"),
            true,
        ),
        _ => (tr("Local model server off"), true),
    }
}

fn dictation_card(ui: &Ui, p: Paint, layer: &mut Vec<El>) -> El {
    let source = ui.settings.dictation_rewrite.source;
    let fields = &ui.model_sources.dictation;
    let mut rows = vec![source_row(p, "dictation", source, |ui, source| {
        ui.update(|settings| settings.dictation_rewrite.source = source);
    })];
    // The speech model is the local transcriber's, so it belongs beside the
    // Local choice; the language applies whichever source cleans the text.
    if source == AiSource::Local {
        rows.push(super::general::speech_model_row(ui, p));
    }
    if let Some(row) = super::general::language_row(ui, p, layer) {
        rows.push(row);
    }
    if source == AiSource::Remote {
        rows.push(endpoint_row(
            p,
            "dictation",
            &fields.endpoint,
            |ui: &mut Ui| &mut ui.model_sources.dictation.endpoint,
        ));
        rows.push(model_row(p, "dictation", &fields.model, |ui: &mut Ui| {
            &mut ui.model_sources.dictation.model
        }));
        rows.push(api_key_row(p, "dictation", &fields.key, |ui: &mut Ui| {
            &mut ui.model_sources.dictation.key
        }));
        if ui.settings.dictation_rewrite.missing().is_some() {
            rows.push(note(
                "dictation-incomplete",
                tr("Remote needs an endpoint that starts with http:// or https:// and a model name"),
                p.warning(),
            ));
        }
    }
    super::controls::section(p, "dictation-model", tr("Dictation"), rows)
}

fn rewrite_card(ui: &Ui, p: Paint) -> El {
    let fields = &ui.model_sources.rewrite;
    let mut rows = vec![
        endpoint_row(p, "rewrite", &fields.endpoint, |ui: &mut Ui| {
            &mut ui.model_sources.rewrite.endpoint
        }),
        model_row(p, "rewrite", &fields.model, |ui: &mut Ui| {
            &mut ui.model_sources.rewrite.model
        }),
        api_key_row(p, "rewrite", &fields.key, |ui: &mut Ui| {
            &mut ui.model_sources.rewrite.key
        }),
    ];
    if ui.settings.rewrite.remote.missing().is_some() {
        rows.push(note(
            "rewrite-incomplete",
            tr("Remote needs an endpoint that starts with http:// or https:// and a model name"),
            p.warning(),
        ));
    }
    rows.push(prompts_row(ui, p));
    super::controls::section(p, "rewrite-model", tr("Rewrite selection"), rows)
}

/// The `Local | Remote` control, which saves as soon as it is pressed.
fn source_row(
    p: Paint,
    key: &'static str,
    source: AiSource,
    pick: impl Fn(&mut Ui, AiSource) + Clone + 'static,
) -> El {
    let selected = usize::from(source == AiSource::Remote);
    let control = segmented(
        p,
        &format!("{key}-source"),
        &[tr("Local"), tr("Remote")],
        selected,
        move |ui, index| {
            pick(
                ui,
                if index == 0 {
                    AiSource::Local
                } else {
                    AiSource::Remote
                },
            );
        },
    );
    setting_row(p, &format!("{key}-run-on"), tr("Run on"), None, control)
}

fn endpoint_row(
    p: Paint,
    feature: &str,
    edit: &TextEdit,
    on_edit: impl for<'a> Fn(&'a mut Ui) -> &'a mut TextEdit + 'static,
) -> El {
    described_row(
        p,
        &format!("{feature}-endpoint"),
        tr("Endpoint"),
        Some((
            tr("For example https://api.example.com/v1").to_string(),
            p.muted(),
        )),
        TextInput::new(edit)
            .placeholder("https://api.example.com/v1")
            .width(Some(FIELD_WIDTH))
            .on_edit(on_edit)
            .build(p.k)
            .key(format!("{feature}-endpoint-field"))
            .label(tr("Endpoint")),
    )
}

fn model_row(
    p: Paint,
    feature: &str,
    edit: &TextEdit,
    on_edit: impl for<'a> Fn(&'a mut Ui) -> &'a mut TextEdit + 'static,
) -> El {
    described_row(
        p,
        &format!("{feature}-model-name"),
        tr("Model"),
        None,
        TextInput::new(edit)
            .placeholder(tr("Model"))
            .width(Some(FIELD_WIDTH))
            .on_edit(on_edit)
            .build(p.k)
            .key(format!("{feature}-model-field"))
            .label(tr("Model")),
    )
}

fn api_key_row(
    p: Paint,
    feature: &str,
    edit: &TextEdit,
    on_edit: impl for<'a> Fn(&'a mut Ui) -> &'a mut TextEdit + 'static,
) -> El {
    described_row(
        p,
        &format!("{feature}-api-key"),
        tr("API key"),
        Some((
            tr("Left empty when the endpoint takes no key").to_string(),
            p.muted(),
        )),
        TextInput::new(edit)
            .secure()
            .width(Some(FIELD_WIDTH))
            .on_edit(on_edit)
            .build(p.k)
            .key(format!("{feature}-key-field"))
            .label(tr("API key")),
    )
}

/// A row that opens the Rewrite prompts sheet and says how many it holds.
fn prompts_row(ui: &Ui, p: Paint) -> El {
    let count = ui.settings.rewrite.snippets.len();
    let summary = match count {
        0 => tr("None").to_string(),
        1 => tr("1 prompt").to_string(),
        count => tr_fill("{} prompts", &count.to_string()),
    };
    setting_row(
        p,
        "rewrite-prompts",
        tr("Prompts"),
        Some(&summary),
        button(
            p,
            "open-prompts",
            tr("Edit"),
            ButtonKind::Secondary,
            |ui: &mut Ui, _| open_prompts(ui),
        ),
    )
}

// ---------------------------------------------------------------------------
// The sheet

fn snippet_row(p: Paint, snippet: &SnippetRow) -> El {
    let id = snippet.id;
    row()
        .key(format!("snippet{id}"))
        .h(ROW_HEIGHT)
        .gap(8.0)
        .center_y()
        .child(
            TextInput::new(&snippet.name)
                .placeholder(tr("Name"))
                .width(Some(200.0))
                .on_edit(move |ui: &mut Ui| ui.model_sources.field(id, false))
                .build(p.k)
                .key("name"),
        )
        .child(
            TextInput::new(&snippet.prompt)
                .placeholder(tr("Instruction"))
                .width(None)
                .on_edit(move |ui: &mut Ui| ui.model_sources.field(id, true))
                .build(p.k)
                .key("instruction")
                .grow(1.0)
                .shrink(),
        )
        .child(remove_button(
            p,
            ControlSize::Regular,
            move |ui: &mut Ui| remove_snippet(ui, id),
        ))
}

pub(super) fn prompts_modal(ui: &Ui, p: Paint) -> Option<[El; 2]> {
    if !ui.model_sources.prompts_open {
        return None;
    }
    let c = &p.k.c;
    let rows = col().key("snippets").gap(8.0).children(
        ui.model_sources
            .snippets
            .iter()
            .map(|snippet| snippet_row(p, snippet)),
    );
    let add = button(
        p,
        "add-prompt",
        tr("Add prompt"),
        ButtonKind::Secondary,
        |ui: &mut Ui, _| add_prompt(ui),
    );
    let footer = row()
        .key("footer")
        .gap(12.0)
        .center_y()
        .child(add)
        .child(rune::el().key("spacer").grow(1.0))
        .child(
            button(
                p,
                "close-prompts",
                tr("Done"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| close_prompts(ui),
            )
            .default_button(),
        );
    let heading = col()
        .key("heading")
        .gap(4.0)
        .child(
            text(tr("Rewrite prompts"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .child(
            text(tr(
                "Say a name alone while text is selected to use its instruction; the default is used when you say nothing",
            ))
            .key("description")
            .font(DESCRIPTION)
            .color(p.muted())
            .wrap(),
        );
    let default_prompt = described_row(
        p,
        "default-prompt",
        tr("Default prompt"),
        None,
        TextInput::new(&ui.model_sources.default_prompt)
            .placeholder(tr("Instruction"))
            .width(Some(MODAL_WIDTH - 2.0 * PADDING - METRICS.row_px * 2.0))
            .on_edit(|ui: &mut Ui| &mut ui.model_sources.default_prompt)
            .build(p.k)
            .key("default-prompt-field")
            .label(tr("Default prompt")),
    );
    let mut panel = col()
        .key("prompts")
        .role("dialog")
        .label(tr("Rewrite prompts"))
        .w(MODAL_WIDTH)
        .pt(PADDING)
        .px(PADDING)
        .pb(14.0)
        .gap(12.0)
        .radius(p.k.radius.xl)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
        .on_press(|_: &mut Ui| {})
        .child(heading)
        .child(default_prompt)
        .child(rows);
    panel = panel.child(footer);
    for shadow in &p.k.elevation.overlay {
        panel = panel.box_shadow(*shadow);
    }
    let layer = |key: &str| {
        rune::el()
            .key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
    };
    Some([
        layer("prompts-scrim")
            .fill(c.scrim)
            .on_press(close_prompts)
            .transition(super::controls::modal_scrim_transition(p)),
        layer("prompts-layer")
            .center()
            .transition(super::controls::modal_transition(p))
            .child(panel.modal(true, close_prompts)),
    ])
}

/// The AI cleanup prompt sheet: one multiline instruction, saved when Done
/// closes it. An empty box is saved as empty and falls back to the shipped
/// wording in the model call.
pub(super) fn cleanup_modal(ui: &Ui, p: Paint) -> Option<[El; 2]> {
    if !ui.model_sources.cleanup_open {
        return None;
    }
    let c = &p.k.c;
    let heading = col()
        .key("heading")
        .gap(4.0)
        .child(
            text(tr("AI cleanup prompt"))
                .key("title")
                .font(p.k.ty.body_medium)
                .color(p.text()),
        )
        .child(
            text(tr(
                "The instruction the model follows for every dictation. Leave it empty to use the shipped wording.",
            ))
            .key("description")
            .font(DESCRIPTION)
            .color(p.muted())
            .wrap(),
        );
    let prompt = TextArea::new(&ui.model_sources.cleanup_prompt)
        .placeholder(tr("Describe how AI cleanup should fix the dictation…"))
        .rows(8)
        .width(None)
        .on_edit(|ui: &mut Ui| &mut ui.model_sources.cleanup_prompt)
        .build(p.k)
        .key("ai-cleanup-prompt-field");
    let footer = row()
        .key("footer")
        .gap(12.0)
        .center_y()
        .child(rune::el().key("spacer").grow(1.0))
        .child(
            button(
                p,
                "close-ai-cleanup-prompt",
                tr("Done"),
                ButtonKind::Secondary,
                |ui: &mut Ui, _| close_cleanup_prompt(ui),
            )
            .default_button(),
        );
    let mut panel = col()
        .key("ai-cleanup-prompt")
        .role("dialog")
        .label(tr("AI cleanup prompt"))
        .w(MODAL_WIDTH)
        .pt(PADDING)
        .px(PADDING)
        .pb(14.0)
        .gap(12.0)
        .radius(p.k.radius.xl)
        .fill(c.bg_base)
        .border(p.hair, c.border_weak)
        .on_press(|_: &mut Ui| {})
        .child(heading)
        .child(note(
            "ai-cleanup-model-limit",
            tr(
                "Results depend on the local model you installed and how large it is. A small model made only for cleaning up English cannot be expected to reliably fix grammar, false starts or abandoned clauses, and it loses a detailed instruction on long text.",
            ),
            p.warning(),
        ))
        .child(prompt)
        .child(footer);
    for shadow in &p.k.elevation.overlay {
        panel = panel.box_shadow(*shadow);
    }
    let layer = |key: &str| {
        rune::el()
            .key(key)
            .role("none")
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
    };
    Some([
        layer("ai-cleanup-scrim")
            .fill(c.scrim)
            .on_press(close_cleanup_prompt)
            .transition(super::controls::modal_scrim_transition(p)),
        layer("ai-cleanup-layer")
            .center()
            .transition(super::controls::modal_transition(p))
            .child(panel.modal(true, close_cleanup_prompt)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui() -> Ui {
        Ui::with(
            AppSettings::default(),
            super::super::Service::Fixture(Vec::new()),
        )
    }

    #[test]
    fn a_field_edit_saves_through_the_draft() {
        let mut ui = ui();
        ui.model_sources.dictation.endpoint = TextEdit::new(" https://api.example.com/v1 ");
        ui.model_sources.dictation.model = TextEdit::new("big-model");
        ui.model_sources.dictation.key = TextEdit::new("secret");
        ui.update(|settings| settings.dictation_rewrite.source = AiSource::Remote);
        ui.flush_drafts();
        assert_eq!(
            ui.settings.dictation_rewrite.remote.endpoint,
            "https://api.example.com/v1"
        );
        assert_eq!(ui.settings.dictation_rewrite.remote.model, "big-model");
        assert_eq!(ui.settings.dictation_rewrite.remote.api_key, "secret");
        assert!(ui.saved > 0, "the edit reached disk");
    }

    #[test]
    fn rewrite_field_edits_save_into_the_remote_model() {
        let mut ui = ui();
        ui.model_sources.rewrite.endpoint = TextEdit::new(" https://api.example.com/v1 ");
        ui.model_sources.rewrite.model = TextEdit::new("big-model");
        ui.model_sources.rewrite.key = TextEdit::new("secret");
        ui.flush_drafts();
        assert_eq!(
            ui.settings.rewrite.remote.endpoint,
            "https://api.example.com/v1"
        );
        assert_eq!(ui.settings.rewrite.remote.model, "big-model");
        assert_eq!(ui.settings.rewrite.remote.api_key, "secret");
        assert!(ui.saved > 0, "the edit reached disk");
    }

    #[test]
    fn the_local_card_names_a_missing_llama_cpp() {
        assert_eq!(
            local_model_copy(Some(crate::local_model::State::Missing)),
            (
                "llama.cpp is not installed, so the local model is off",
                true
            )
        );
        assert_eq!(
            local_model_copy(Some(crate::local_model::State::Ready)),
            ("Local model server running", false)
        );
        assert_eq!(
            local_model_copy(Some(crate::local_model::State::Failed)),
            ("Local model server failed", true)
        );
        assert_eq!(local_model_copy(None), ("Local model server off", true));
    }

    #[test]
    fn the_local_card_says_it_serves_the_ai_cleanup() {
        assert!(LOCAL_MODEL_PURPOSE.contains("AI cleanup card"));
        assert!(LOCAL_MODEL_PURPOSE.contains("Dictation is set to Local"));
        assert!(LOCAL_MODEL_PURPOSE.contains("starts it with the app"));
        // The Modes pane is hidden (`Pane::Modes` is never shown), so naming
        // it sent people to a surface that is not in the app.
        assert!(!LOCAL_MODEL_PURPOSE.contains("Modes"));
    }

    #[test]
    fn the_sheet_projects_names_instructions_and_the_default_prompt() {
        let mut ui = ui();
        open_prompts(&mut ui);
        assert_eq!(ui.model_sources.snippets.len(), 1);
        ui.model_sources.snippets[0].name = TextEdit::new("structured prompt");
        ui.model_sources.snippets[0].prompt = TextEdit::new("Turn it into a prompt.");
        ui.model_sources.default_prompt = TextEdit::new("Tighten it.");
        close_prompts(&mut ui);
        assert_eq!(ui.settings.rewrite.snippets.len(), 1);
        assert_eq!(ui.settings.rewrite.snippets[0].name, "structured prompt");
        assert_eq!(
            ui.settings.rewrite.snippets[0].prompt,
            "Turn it into a prompt."
        );
        assert_eq!(ui.settings.rewrite.default_prompt, "Tighten it.");
    }

    #[test]
    fn the_cleanup_prompt_sheet_projects_the_saved_prompt() {
        let mut ui = ui();
        open_cleanup_prompt(&mut ui);
        assert!(ui.model_sources.cleanup_open);
        ui.model_sources.cleanup_prompt = TextEdit::new("  Fix the words.  ");
        close_cleanup_prompt(&mut ui);
        assert_eq!(ui.settings.ai_cleanup.prompt, "Fix the words.");

        open_cleanup_prompt(&mut ui);
        ui.model_sources.cleanup_prompt = TextEdit::new("   ");
        close_cleanup_prompt(&mut ui);
        assert_eq!(ui.settings.ai_cleanup.prompt, "");
    }

    #[test]
    fn a_blank_snippet_is_left_out() {
        let mut editor = ModelSources::new(&AppSettings::default());
        let id = editor.take_id();
        editor.snippets.push(SnippetRow {
            id,
            name: TextEdit::new("   "),
            prompt: TextEdit::new("Do it."),
        });
        let mut settings = AppSettings::default();
        editor.project(&mut settings);
        assert!(settings.rewrite.snippets.is_empty());
    }

    #[test]
    fn a_late_edit_for_a_removed_snippet_touches_nothing() {
        let mut ui = ui();
        open_prompts(&mut ui);
        let id = ui.model_sources.snippets[0].id;
        ui.model_sources.snippets[0].name = TextEdit::new("kept");
        ui.model_sources.snippets[0].prompt = TextEdit::new("Kept.");
        ui.flush_drafts();
        let kept = ui.settings.rewrite.snippets.len();
        remove_snippet(&mut ui, id);
        ui.model_sources.field(id, false).text = "late".into();
        ui.model_sources.field(id, true).text = "late".into();
        assert_eq!(ui.settings.rewrite.snippets.len(), kept.saturating_sub(1));
    }

    #[test]
    fn the_pane_and_the_sheet_build() {
        let mut ui = ui();
        let p = super::super::controls::Paint::new(true, 1.0);
        let mut layer = Vec::new();
        let (actions, _body) = pane(&ui, p, &mut layer);
        assert!(actions.is_empty());
        assert!(prompts_modal(&ui, p).is_none());

        open_prompts(&mut ui);
        assert!(prompts_modal(&ui, p).is_some());

        // The Dictation card showing its remote fields builds too.
        ui.settings.dictation_rewrite.source = AiSource::Remote;
        let mut layer = Vec::new();
        let _ = pane(&ui, p, &mut layer);
    }

    #[test]
    fn the_pane_is_listed_with_a_unique_key() {
        use super::super::Pane;
        assert_eq!(Pane::Models.key(), "nav-models");
        assert_eq!(Pane::Models.title(), "Models");
    }
}
