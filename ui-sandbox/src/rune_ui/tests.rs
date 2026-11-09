//! The sandbox's accessibility tests: the General pane's order and grouping,
//! ported from the app's `src/rune_ui/accessibility_tests.rs`.

use rune::text::TextSystem;
use rune::{Env, Runtime};
use serde_json::Value;

use super::*;

const SIZE: (f32, f32) = (980.0, 760.0);

fn runtime() -> Runtime<Ui> {
    let state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
    let env = Env {
        system_dark: true,
        scale: 2.0,
    };
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = rune::Runtime::with_text(state, view, SIZE, env, text);
    rt.set_srgb_blend(true);
    rt.poll = Some(Ui::poll);
    rt.on_key = Some(Ui::on_key);
    rt.settle_all();
    rt.frame(0.0);
    rt
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

/// General reads the hardware first, then the keys with their mode and cancel,
/// then Vocabulary and Launch at login.
#[test]
#[cfg(target_os = "windows")]
fn general_reads_the_hardware_then_the_keys_then_the_rest() {
    let mut rt = runtime();
    rt.state.settings.shortcut_mode = crate::dictation::ShortcutMode::Toggle;
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
        "launch-at-login",
    ];
    let indices: Vec<usize> = order.iter().map(|key| index(key)).collect();
    assert!(
        indices.windows(2).all(|pair| pair[0] < pair[1]),
        "General reads out of order: {order:?} at {indices:?}"
    );
    // The one card groups its rows by air: each spacer sits below the group
    // it follows and above the group it introduces.
    let rect = |key: &str| {
        rt.presented(key)
            .unwrap_or_else(|| panic!("{key} is not laid out"))
            .0
    };
    let boundary = |above: &str, space: &str, below: &str| {
        let [_, above_top, _, above_height] = rect(above);
        let [_, space_top, _, space_height] = rect(space);
        let [_, below_top, _, _] = rect(below);
        assert!(
            space_top >= above_top + above_height,
            "{space} starts after {above}: {space_top} vs {}",
            above_top + above_height
        );
        assert!(space_height >= 8.0, "{space} carries air: {space_height}");
        assert!(
            below_top >= space_top + space_height,
            "{below} starts after {space}: {below_top} vs {}",
            space_top + space_height
        );
    };
    boundary(
        "microphone-mode",
        "general-space-keys",
        "dictation-shortcut",
    );
    boundary(
        "rewrite-shortcut",
        "general-space-shortcut-mode",
        "shortcut-mode",
    );
    boundary("cancel-shortcut", "general-space-vocabulary", "vocabulary");
    boundary("vocabulary", "general-space-launch", "launch-at-login");
}
