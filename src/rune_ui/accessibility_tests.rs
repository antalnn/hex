//! The window as screen readers and automation read it: the accessibility
//! tree Rune builds from the panes (`docs/features/accessibility.md`).
//! Every control a person can reach has a name, switches say whether they
//! are on, choices say what they choose and their value, the sidebar says
//! which pane shows, and lists say how long they are.

use rune::accesskit::Action;
use rune::text::TextSystem;
use rune::{Env, Runtime, TextEdit};
use serde_json::Value;

use super::*;

const SIZE: (f32, f32) = (980.0, 760.0);

fn runtime(scene: &str) -> Runtime<Ui> {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    let (_, apply) = SCENES
        .iter()
        .find(|(name, _)| *name == scene)
        .unwrap_or_else(|| panic!("no scene {scene}"));
    apply(&mut state);
    let env = Env {
        system_dark: true,
        scale: 2.0,
    };
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = offscreen::runtime(state, view, SIZE, env, text);
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    rt.settle_all();
    rt.frame(0.0);
    rt
}

fn settle(rt: &mut Runtime<Ui>) {
    for _ in 0..90 {
        rt.frame(1.0 / 30.0);
    }
    rt.settle_all();
    rt.frame(0.0);
}

/// The Models pane, where the speech model and language rows live now.
fn open_models(rt: &mut Runtime<Ui>) {
    rt.state.pane = Pane::Models;
    rt.settle_all();
    rt.frame(0.0);
}

/// Every node of `tree` with the path of keys and roles above it.
fn nodes(tree: &Value) -> Vec<(String, &Value)> {
    fn walk<'a>(node: &'a Value, path: String, out: &mut Vec<(String, &'a Value)>) {
        let step = node["key"].as_str().map_or_else(
            || node["role"].as_str().unwrap_or("?").to_owned(),
            str::to_owned,
        );
        let path = if path.is_empty() {
            step
        } else {
            format!("{path}/{step}")
        };
        out.push((path.clone(), node));
        for child in node["children"].as_array().into_iter().flatten() {
            walk(child, path.clone(), out);
        }
    }
    let mut out = Vec::new();
    walk(tree, String::new(), &mut out);
    out
}

/// The node keyed `key` in `rt`'s accessibility tree.
pub(super) fn node(rt: &Runtime<Ui>, key: &str) -> Value {
    let tree = rt.accessibility_json();
    let all = nodes(&tree);
    all.iter()
        .find(|(_, node)| node["key"].as_str() == Some(key))
        .map(|(_, node)| (*node).clone())
        .unwrap_or_else(|| {
            let paths: Vec<&str> = all.iter().map(|(path, _)| path.as_str()).collect();
            panic!(
                "no accessible element keyed {key}; drawn at {:?}; the tree:\n{}",
                rt.find(key),
                paths.join("\n")
            )
        })
}

/// The accessible value of the element keyed `key`: a choice's chosen item.
pub(super) fn a11y_value(rt: &Runtime<Ui>, key: &str) -> Option<String> {
    node(rt, key)["value"].as_str().map(str::to_owned)
}

fn states(node: &Value) -> Vec<&str> {
    node["states"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn name(node: &Value) -> &str {
    node["name"].as_str().unwrap_or_default().trim()
}

/// Roles a person operates, so assistive technology must say what they do.
const CONTROLS: &[&str] = &[
    "button",
    "default-button",
    "check-box",
    "switch",
    "radio-button",
    "combo-box",
    "text-input",
    "search-input",
    "multiline-text-input",
    "slider",
    "link",
    "menu-item",
    "menu-item-check-box",
    "menu-item-radio",
    "list-box-option",
    "list-item",
    "tab",
];

/// A control with no name is a control a screen reader user can't use, and
/// one automation can't find by what it says.
#[test]
fn every_control_in_every_scene_has_a_name() {
    let mut unnamed = Vec::new();
    for (scene, _) in SCENES.iter().filter(|(name, _)| *name != "live") {
        let rt = runtime(scene);
        let tree = rt.accessibility_json();
        for (path, node) in nodes(&tree) {
            let role = node["role"].as_str().unwrap_or_default();
            let acts = node["actions"]
                .as_array()
                .is_some_and(|actions| actions.iter().any(|a| a == "click" || a == "focus"));
            if (acts || CONTROLS.contains(&role)) && name(node).is_empty() {
                unnamed.push(format!("{scene}: {path} ({role})"));
            }
        }
    }
    assert!(
        unnamed.is_empty(),
        "controls without a name:\n{}",
        unnamed.join("\n")
    );
}

/// The window's tree starts at the pane's title, a heading.
#[test]
fn the_pane_title_is_its_heading() {
    let rt = runtime("history");
    let title = node(&rt, "pane-title");
    assert_eq!(title["role"], "heading");
    assert_eq!(title["level"], 1);
    assert_eq!(name(&title), "History");
}

/// The sidebar is a list whose shown pane is selected, and choosing
/// another through accessibility switches to it, as a click does.
#[test]
fn the_sidebar_says_which_pane_shows_and_switches_through_accessibility() {
    let mut rt = runtime("portal");
    let general = node(&rt, "nav-general");
    assert_eq!(general["role"], "list-item");
    assert_eq!(name(&general), "General");
    assert!(states(&general).contains(&"selected"), "{general}");
    assert!(!states(&node(&rt, "nav-history")).contains(&"selected"));

    assert!(rt.accessibility_action("nav-history", Action::Click, None));
    settle(&mut rt);
    assert_eq!(rt.state.pane, Pane::History);
    assert!(states(&node(&rt, "nav-history")).contains(&"selected"));
    assert!(!states(&node(&rt, "nav-general")).contains(&"selected"));
    assert_eq!(name(&node(&rt, "pane-title")), "History");
}

/// History's list says how many entries it holds and each row its place,
/// its text, and what made it.
#[test]
fn history_lists_its_entries_with_their_place() {
    let rt = runtime("history");
    let list = node(&rt, "history-list");
    assert_eq!(list["role"], "list");
    assert_eq!(name(&list), "History");
    assert_eq!(list["setSize"], 3);
    let first = node(&rt, "history-entry3");
    assert_eq!(first["role"], "list-item");
    assert_eq!(first["position"], 1);
    assert_eq!(
        name(&first),
        "Let's move the standup to ten thirty tomorrow."
    );
    assert!(
        first["description"]
            .as_str()
            .is_some_and(|description| description.starts_with("Dictation · WezTerm")),
        "{first}"
    );
    assert!(states(&node(&rt, "history-entry2")).contains(&"selected"));
    assert_eq!(node(&rt, "history-entry1")["position"], 3);
}

/// A settings switch is the whole row: named by its title, described by its
/// note, and on or off; pressing it through accessibility flips it.
#[test]
fn settings_switches_say_whether_they_are_on() {
    let mut rt = runtime("portal");
    let release = rt.state.settings.release_microphone_while_idle;
    let row = node(&rt, "microphone-mode");
    assert_eq!(row["role"], "switch");
    assert_eq!(name(&row), "Turn off the mic when not dictating");
    assert!(row["description"].as_str().is_some_and(|d| !d.is_empty()));
    let on = |row: &Value| states(row).contains(&"checked");
    assert_eq!(on(&row), release);

    assert!(rt.accessibility_action("microphone-mode", Action::Click, None));
    settle(&mut rt);
    assert_eq!(rt.state.settings.release_microphone_while_idle, !release);
    assert_eq!(on(&node(&rt, "microphone-mode")), !release);
}

/// The local model server's controls live on the Models pane only.
#[test]
fn the_local_model_controls_live_on_the_models_pane_only() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    assert!(
        rt.find("local-model-start").is_some() || rt.find("local-model-stop").is_some(),
        "the Models pane keeps the server control"
    );

    rt.state.pane = Pane::Output;
    rt.settle_all();
    rt.frame(0.0);
    assert!(rt.find("local-model-start").is_none());
    assert!(rt.find("local-model-stop").is_none());
    assert!(rt.find("local-model-state").is_none());
}

/// The local model card says what the server is for.
#[test]
fn the_local_model_card_says_what_it_serves() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    assert!(
        rt.find("local-model-purpose").is_some(),
        "the Models pane states the local model's job"
    );
}

/// General reads the hardware first, then the keys with their mode, then
/// Vocabulary, one card per section. Under Advanced, Dictation puts the
/// Speech engine above the Bluetooth switch, and Application carries the
/// launch toggle and Launch at login together.
#[test]
#[cfg(target_os = "windows")]
fn general_reads_the_hardware_then_the_keys_then_the_rest() {
    let mut rt = runtime("portal");
    rt.state.settings.shortcut_mode = crate::dictation::ShortcutMode::Toggle;
    rt.state.advanced = vec![Pane::General];
    rt.invalidate();
    rt.settle_all();
    rt.frame(0.0);
    let tree = rt.accessibility_json();
    let all = nodes(&tree);
    let index = |key: &str| {
        all.iter()
            .position(|(_, node)| node["key"].as_str() == Some(key))
            .unwrap_or_else(|| panic!("{key} is missing"))
    };
    let order = [
        "microphone",
        "microphone-mode",
        "dictation-shortcut",
        "rewrite-shortcut",
        "shortcut-mode",
        "cancel-shortcut",
        "vocabulary",
        "speech-engine",
        "avoid-bluetooth-microphone",
        "enable-on-launch",
        "launch-at-login",
    ];
    let indices: Vec<usize> = order.iter().map(|key| index(key)).collect();
    assert!(
        indices.windows(2).all(|pair| pair[0] < pair[1]),
        "General reads out of order: {order:?} at {indices:?}"
    );
    // Every section opens with its heading, and its rows follow before the
    // next section's heading.
    let headings = [
        "Microphone",
        "Keys",
        "Shortcut mode",
        "Vocabulary",
        "Dictation",
        "Output",
        "Application",
    ];
    let heading = |title: &str| {
        all.iter()
            .position(|(_, node)| {
                node["role"].as_str() == Some("heading")
                    && node["name"].as_str().map(str::trim) == Some(title)
            })
            .unwrap_or_else(|| panic!("the {title} heading is missing"))
    };
    let heading_indices: Vec<usize> = headings.iter().map(|title| heading(title)).collect();
    assert!(
        heading_indices.windows(2).all(|pair| pair[0] < pair[1]),
        "General's sections read out of order: {headings:?} at {heading_indices:?}"
    );
    let sections: [(&str, &[&str]); 7] = [
        ("Microphone", &["microphone", "microphone-mode"]),
        ("Keys", &["dictation-shortcut", "rewrite-shortcut"]),
        ("Shortcut mode", &["shortcut-mode", "cancel-shortcut"]),
        ("Vocabulary", &["vocabulary"]),
        (
            "Dictation",
            &["speech-engine", "avoid-bluetooth-microphone"],
        ),
        ("Output", &[]),
        ("Application", &["enable-on-launch", "launch-at-login"]),
    ];
    for (position, (title, rows)) in sections.iter().enumerate() {
        let start = heading_indices[position];
        let end = heading_indices
            .get(position + 1)
            .copied()
            .unwrap_or(all.len());
        for row in *rows {
            let row = index(row);
            assert!(
                row > start && row < end,
                "{row}({row}) does not sit under the {title} heading at {start}"
            );
        }
    }
    // The toggle names dictation, and Launch at login sits directly under it.
    let toggle = index("enable-on-launch");
    assert_eq!(
        all[toggle].1["name"].as_str(),
        Some("Enable dictation on launch")
    );
    assert!(
        index("launch-at-login") > toggle,
        "Launch at login does not follow the launch toggle"
    );
}

/// The Models pane puts the features before the server that serves one.
#[test]
fn models_reads_the_features_then_the_server() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    let tree = rt.accessibility_json();
    let all = nodes(&tree);
    let index = |key: &str| {
        all.iter()
            .position(|(_, node)| node["key"].as_str() == Some(key))
            .unwrap_or_else(|| panic!("{key} is missing"))
    };
    let order = [
        "rewrite-endpoint-field",
        "ai-cleanup-enabled",
        "ai-cleanup-prompt",
        "ai-cleanup-run-on",
        "ai-cleanup-source",
        "local-model-auto-start",
        "local-model-state",
        "local-model-files",
    ];
    let indices: Vec<usize> = order.iter().map(|key| index(key)).collect();
    assert!(
        indices.windows(2).all(|pair| pair[0] < pair[1]),
        "Models reads out of order: {order:?} at {indices:?}"
    );
}

/// AI cleanup reads as its own card: a switch that flips through
/// accessibility, the prompt row that opens its sheet, and where it runs,
/// all between Rewrite selection and the local model server.
#[test]
fn ai_cleanup_reads_its_switch_prompt_and_server() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    let switch = node(&rt, "ai-cleanup-enabled");
    assert_eq!(switch["role"], "switch");
    assert_eq!(name(&switch), "AI cleanup");
    assert!(states(&switch).contains(&"checked"));
    assert!(
        switch["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()),
        "{switch}"
    );
    assert!(rt.find("ai-cleanup-prompt").is_some());
    assert!(
        rt.find("open-ai-cleanup-prompt").is_some(),
        "the prompt opens its own sheet"
    );
    assert!(rt.find("ai-cleanup-runs").is_some());

    let auto_start = node(&rt, "local-model-auto-start");
    assert_eq!(auto_start["role"], "switch");
    assert_eq!(name(&auto_start), "Start with Hex");
    assert!(states(&auto_start).contains(&"checked"));

    assert!(rt.accessibility_action("ai-cleanup-enabled", Action::Click, None));
    settle(&mut rt);
    assert!(!rt.state.settings.ai_cleanup.enabled);
    assert!(!states(&node(&rt, "ai-cleanup-enabled")).contains(&"checked"));

    assert!(rt.accessibility_action("open-ai-cleanup-prompt", Action::Click, None));
    settle(&mut rt);
    let warning = node(&rt, "ai-cleanup-model-limit");
    assert!(
        warning["children"][0]["name"].as_str().is_some_and(
            |name| name.contains("How well this works depends on how large the local model")
        ),
        "{warning}"
    );
}

/// Rewrite selection always calls a remote endpoint: the Models pane shows its
/// address fields and no Local choice, while AI cleanup keeps the one Run on
/// choice the dictation cleanup reads.
#[test]
fn rewrite_selection_calls_a_remote_endpoint_only() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    assert!(
        rt.find("ai-cleanup-source").is_some(),
        "AI cleanup carries the Local or Remote choice"
    );
    assert!(
        rt.find("rewrite-source").is_none(),
        "Rewrite selection has no Local choice"
    );
    assert!(
        rt.find("rewrite-endpoint-field").is_some(),
        "the remote endpoint is editable"
    );
    assert!(rt.find("rewrite-model-field").is_some());
    assert!(rt.find("rewrite-key-field").is_some());
    assert!(
        rt.find("dictation-endpoint-field").is_none(),
        "AI cleanup starts on Local"
    );
}

/// A publish made before the window's save carries the old source; it must
/// not flip Run on back. The remote fields are editors and survive by the
/// draft check, but the source is a direct setting and had no protection.
#[test]
fn a_stale_publish_cannot_undo_the_run_on_choice() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    let stale = rt.state.settings.clone();
    rt.state.adopt(stale.clone());
    super::ai_models::set_dictation_source(&mut rt.state, crate::app_settings::AiSource::Remote);
    let mut older = stale;
    older.enable_on_launch = !older.enable_on_launch;
    rt.state.adopt(older);
    assert_eq!(
        rt.state.model_sources.dictation_source,
        crate::app_settings::AiSource::Remote,
        "the pre-save publish flipped the control's draft back"
    );
    rt.state.flush_drafts();
    assert_eq!(
        rt.state.settings.dictation_rewrite.source,
        crate::app_settings::AiSource::Remote,
        "the choice is saved again after the stale publish"
    );
}

/// Typing into the AI cleanup prompt sheet and pressing Done saves the typed
/// instruction, not the shipped one it opened with. The text field takes the
/// text through its accessibility `set-value`, the same path automation uses.
#[test]
fn the_cleanup_prompt_sheet_saves_typed_text() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    assert!(rt.accessibility_action("open-ai-cleanup-prompt", Action::Click, None));
    settle(&mut rt);
    assert!(rt.accessibility_action(
        "input",
        Action::SetValue,
        Some(rune::accesskit::ActionData::Value(
            "Reply with two short bullet points and nothing else".into()
        ))
    ));
    settle(&mut rt);
    assert!(rt.accessibility_action("close-ai-cleanup-prompt", Action::Click, None));
    settle(&mut rt);
    assert_eq!(
        rt.state.settings.ai_cleanup.prompt,
        "Reply with two short bullet points and nothing else"
    );
}

/// Typing the AI cleanup prompt is a draft like any other: a publish that
/// arrives before it saves must not reload the sheet's text from the old
/// settings.
#[test]
fn an_ai_cleanup_prompt_draft_survives_a_publish() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    let stale = rt.state.settings.clone();
    rt.state.adopt(stale.clone());
    rt.state.model_sources.cleanup_prompt =
        TextEdit::new("Reply with two short bullet points and nothing else");
    let mut older = stale;
    older.rewrite.remote.model = "another-model".into();
    rt.state.adopt(older);
    assert_eq!(
        rt.state.model_sources.cleanup_prompt.text,
        "Reply with two short bullet points and nothing else"
    );
}

/// The AI cleanup card's Run on controls read `dictation_rewrite.source`, the
/// setting the dictation cleanup already uses: Remote swaps the local server
/// controls for the endpoint, model and key rows.
#[test]
fn the_ai_cleanup_card_reads_the_reused_source_setting() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    assert!(rt.find("ai-cleanup-run-on").is_some());
    assert!(rt.find("ai-cleanup-deadline-field").is_some());
    assert!(rt.find("local-model-auto-start").is_some());
    assert!(rt.find("local-model-state").is_some());

    rt.state.settings.dictation_rewrite.source = crate::app_settings::AiSource::Remote;
    rt.state.model_sources.reload(&rt.state.settings);
    settle(&mut rt);
    assert!(rt.find("dictation-endpoint-field").is_some());
    assert!(rt.find("dictation-model-field").is_some());
    assert!(rt.find("dictation-key-field").is_some());
    assert!(
        rt.find("local-model-auto-start").is_none(),
        "the local controls hide while Remote runs"
    );
    assert!(rt.find("local-model-state").is_none());
    assert!(
        rt.find("local-model-section").is_none(),
        "the Local model section hides while Remote runs"
    );
    assert!(rt.find("local-model-files").is_none());
}

/// The speech model and language stay on Dictation whichever source cleans
/// the text, because the model transcribes and the source only cleans;
/// General keeps neither row.
#[test]
fn the_dictation_card_holds_the_speech_model_and_language() {
    let mut rt = runtime("portal");
    rt.state.settings.transcription.model =
        crate::transcription::models::TranscriptionModelId::WhisperLargeV3Turbo;
    rt.state.settings.transcription.language = "en".into();
    open_models(&mut rt);
    let language_row = |rt: &Runtime<Ui>| {
        rt.find("transcription-language-control").is_some()
            || rt.find("transcription-language-control-trigger").is_some()
    };
    assert!(rt.find("transcription-control").is_some());
    assert!(language_row(&rt));

    rt.state.settings.dictation_rewrite.source = crate::app_settings::AiSource::Remote;
    rt.settle_all();
    rt.frame(0.0);
    assert!(
        rt.find("transcription-control").is_some(),
        "the speech model transcribes whichever source cleans"
    );
    assert!(language_row(&rt), "Language shows for Remote too");

    rt.state.pane = Pane::General;
    rt.settle_all();
    rt.frame(0.0);
    assert!(rt.find("transcription-control").is_none());
    assert!(rt.find("transcription-language-control").is_none());
}

/// A choice reads as what it chooses, with the current choice as its value.
#[test]
fn choices_name_what_they_choose_and_read_their_value() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    let model = node(&rt, "transcription-control");
    assert_eq!(name(&model), "Speech model");
    assert_eq!(
        a11y_value(&rt, "transcription-control").as_deref(),
        Some("Parakeet Unified English")
    );
    // The microphone still lives on General.
    rt.state.pane = Pane::General;
    rt.settle_all();
    rt.frame(0.0);
    let microphone = node(&rt, "microphone-control");
    assert_eq!(name(&microphone), "Microphone");
    assert!(microphone["value"].as_str().is_some_and(|v| !v.is_empty()));
}

/// The model picker is a dialog named by its title, the only part of the
/// window a screen reader reaches while it is open.
#[test]
fn the_model_picker_is_a_named_modal_dialog() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    assert!(rt.accessibility_action("transcription-control", Action::Click, None));
    settle(&mut rt);
    assert!(rt.state.picker.open);
    let picker = node(&rt, "transcription-picker");
    assert_eq!(picker["role"], "dialog");
    assert!(!name(&picker).is_empty());
    assert!(states(&picker).contains(&"modal"), "{picker}");
    let tree = rt.accessibility_json();
    assert!(
        !nodes(&tree)
            .iter()
            .any(|(_, node)| node["key"] == "nav-general"),
        "the window behind the picker leaves the tree"
    );
}

/// Closing Settings and opening it again keeps the window's runtime but
/// gives it a new accessibility adapter. With assistive technology on, the
/// new adapter's first update must hold the whole tree: a partial one
/// panicked AccessKit, which took the window down on reopen.
#[test]
fn settings_reopened_sends_its_whole_tree_to_the_new_adapter() {
    let mut rt = runtime("portal");
    open_models(&mut rt);
    let mut first = rune::AccessibilityLink::new();
    first.activate();
    let opened = first
        .service(&mut rt, "Hex")
        .expect("the first window's tree");
    assert!(opened.tree.is_some());
    assert!(rt.accessibility_action("transcription-control", Action::Click, None));
    settle(&mut rt);
    assert!(
        first
            .service(&mut rt, "Hex")
            .expect("the picker opened")
            .tree
            .is_none()
    );

    rt.update(on_close);
    settle(&mut rt);
    drop(first);

    rt.update(|ui, tx| show(ui, Destination::Settings, tx));
    settle(&mut rt);
    let mut reopened = rune::AccessibilityLink::new();
    if let Some(update) = reopened.service(&mut rt, "Hex") {
        assert!(
            update.tree.is_some(),
            "a partial tree before the client asked"
        );
    }
    reopened.activate();
    let update = reopened
        .service(&mut rt, "Hex")
        .expect("the reopened window's tree");
    assert!(update.tree.is_some());
    assert!(
        update
            .nodes
            .iter()
            .any(|(_, node)| node.author_id() == Some("microphone-control")),
        "the reopened window shows Settings"
    );
}
