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

/// General reads the hardware first, then the keys with their mode, then
/// Vocabulary, one card per section. Under Advanced, Dictation puts the
/// Speech engine above the Bluetooth switch, and Application carries the
/// launch toggle and Launch at login together.
#[test]
#[cfg(target_os = "windows")]
fn general_reads_the_hardware_then_the_keys_then_the_rest() {
    let mut rt = runtime();
    rt.state.settings.shortcut_mode = crate::dictation::ShortcutMode::Toggle;
    rt.state.advanced = vec![Pane::General];
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

/// `HEX_SANDBOX_SNAPS=<folder>` writes the Models pane's AI cleanup card,
/// light and dark, with the prompt sheet closed and open.
#[test]
#[cfg(target_os = "windows")]
fn models_pane_snapshots() {
    let Some(folder) = std::env::var_os("HEX_SANDBOX_SNAPS") else {
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let Ok(gpu) = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None) else {
        eprintln!("skipping Models snapshots: no GPU adapter");
        return;
    };
    let size = (980.0, 1500.0);
    let scale = 2.0;
    for (label, prompt_open, setup_open, remote, enabled) in [
        ("ai-cleanup", false, false, false, true),
        ("ai-cleanup-off", false, false, false, false),
        ("ai-cleanup-prompt", true, false, false, true),
        ("ai-cleanup-setup", false, true, false, true),
        ("ai-cleanup-setup-remote", false, true, true, true),
    ] {
        for dark in [false, true] {
            let mut settings = AppSettings::default();
            settings.ai_cleanup.enabled = enabled;
            if remote {
                settings.dictation_rewrite.source = crate::app_settings::AiSource::Remote;
                settings.dictation_rewrite.remote.endpoint =
                    "https://opencode.ai/inference/go/openai/v1/chat/completions".into();
                settings.dictation_rewrite.remote.model = "muse-spark-1.3-contributor".into();
            }
            let mut state = Ui::with(settings, Service::Fixture(Vec::new()));
            state.model_sources.reload(&state.settings);
            let env = Env {
                system_dark: dark,
                scale,
            };
            let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
            let mut rt = Runtime::with_text(state, view, size, env, text);
            rt.set_srgb_blend(true);
            rt.poll = Some(Ui::poll);
            rt.on_key = Some(Ui::on_key);
            rt.state.pane = Pane::Models;
            rt.state.model_sources.cleanup_open = prompt_open;
            rt.state.model_sources.setup_open = setup_open;
            let width = (size.0 * scale) as u32;
            let height = (size.1 * scale) as u32;
            let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, width, height);
            let theme = if dark { "dark" } else { "light" };
            let path = folder.join(format!("models-{label}-{theme}.png"));
            rune::render::write_png(&path, width, height, &pixels).unwrap();
            println!("wrote {}", path.display());
        }
    }
}

/// `HEX_SANDBOX_SNAPS=<folder>` writes the General pane with its Advanced
/// cards open, light and dark, tall enough to show the changed rows.
#[test]
#[cfg(target_os = "windows")]
fn general_pane_snapshots() {
    let Some(folder) = std::env::var_os("HEX_SANDBOX_SNAPS") else {
        return;
    };
    let folder = std::path::PathBuf::from(folder);
    std::fs::create_dir_all(&folder).unwrap();
    let Ok(gpu) = rune::render::Gpu::try_new(rune::render::Gpu::instance(), None) else {
        eprintln!("skipping General snapshots: no GPU adapter");
        return;
    };
    let size = (980.0, 1900.0);
    let scale = 2.0;
    for dark in [false, true] {
        let state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()));
        let env = Env {
            system_dark: dark,
            scale,
        };
        let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
        let mut rt = Runtime::with_text(state, view, size, env, text);
        rt.set_srgb_blend(true);
        rt.poll = Some(Ui::poll);
        rt.on_key = Some(Ui::on_key);
        rt.state.advanced = vec![Pane::General];
        let width = (size.0 * scale) as u32;
        let height = (size.1 * scale) as u32;
        let pixels = crate::rune_ui::offscreen::draw(&gpu, &mut rt, width, height);
        let theme = if dark { "dark" } else { "light" };
        let path = folder.join(format!("general-rows-{theme}.png"));
        rune::render::write_png(&path, width, height, &pixels).unwrap();
        println!("wrote {}", path.display());
    }
}
