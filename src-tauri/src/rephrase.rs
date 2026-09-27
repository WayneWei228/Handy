//! Native rephrase panel.
//!
//! When `native_rephrase_enabled` is on, a finished dictation is not pasted
//! right away. Instead the rephrase panel shows the transcript next to a
//! native-speaker rewrite from the active post-processing provider. The user
//! moves between the two rows with ↑/↓ and pastes the selected one by pressing
//! the transcribe shortcut again; Esc dismisses the panel without pasting.
//!
//! The panel never takes focus. ↑/↓/Esc are registered as dynamic shortcuts
//! (swallowed by the hotkey backend) only while the panel is open, the same way
//! the cancel shortcut is registered only while recording.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use log::{debug, error, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::actions::{strip_invisible_chars, strip_think_block};
use crate::settings::{get_settings, AppSettings, ShortcutBinding, APPLE_INTELLIGENCE_PROVIDER_ID};
use crate::shortcut;
use crate::transcription_coordinator::is_transcribe_binding;
use crate::utils;

pub const REPHRASE_UP_BINDING: &str = "rephrase_up";
pub const REPHRASE_DOWN_BINDING: &str = "rephrase_down";
pub const REPHRASE_DISMISS_BINDING: &str = "rephrase_dismiss";

const REPHRASE_TIMEOUT: Duration = Duration::from_secs(15);

const SYSTEM_PROMPT: &str = "You rewrite English sentences so they sound like a fluent native \
speaker said them. Keep the original meaning, tone, and level of formality. Fix grammar, word \
choice, and unnatural phrasing. Do not add new information. Output only the rewritten text, with \
no quotes, labels, or explanations.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Row {
    Original,
    Native,
}

/// Snapshot sent to the panel webview on every change.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RephraseView {
    pub original: String,
    pub native: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub already_natural: bool,
    pub selected: Row,
}

#[derive(Debug)]
struct PendingChoice {
    id: u64,
    view: RephraseView,
}

impl PendingChoice {
    fn native_selectable(&self) -> bool {
        self.view.native.is_some() && !self.view.already_natural
    }
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    pending: Option<PendingChoice>,
    /// Transcribe binding whose key release should be ignored because its
    /// press pasted the choice.
    swallow_release_for: Option<String>,
}

/// Pure state for the panel; no Tauri calls, so it is unit-testable.
#[derive(Default)]
pub struct RephraseState {
    inner: Mutex<Inner>,
    /// Serializes register/unregister of the panel keys and records whether
    /// they are currently registered.
    keys_registered: Mutex<bool>,
}

impl RephraseState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start a new pending choice for `original`. Returns its id.
    pub fn open(&self, original: String) -> (u64, RephraseView) {
        let mut inner = self.lock();
        inner.next_id += 1;
        let id = inner.next_id;
        let view = RephraseView {
            original,
            native: None,
            loading: true,
            error: None,
            already_natural: false,
            selected: Row::Original,
        };
        inner.pending = Some(PendingChoice {
            id,
            view: view.clone(),
        });
        (id, view)
    }

    /// Store the rephrase result for choice `id`. Returns the new view, or
    /// `None` if that choice is no longer pending.
    pub fn set_result(&self, id: u64, result: Result<String, String>) -> Option<RephraseView> {
        let mut inner = self.lock();
        let pending = inner.pending.as_mut().filter(|p| p.id == id)?;
        pending.view.loading = false;
        match result {
            Ok(native) => {
                pending.view.already_natural = is_already_natural(&pending.view.original, &native);
                pending.view.native = Some(native);
                pending.view.selected = if pending.view.already_natural {
                    Row::Original
                } else {
                    Row::Native
                };
            }
            Err(err) => {
                pending.view.error = Some(err);
                pending.view.selected = Row::Original;
            }
        }
        Some(pending.view.clone())
    }

    /// Move the selection up (towards the original) or down (towards the
    /// native rewrite). Returns the new view if a choice is pending.
    pub fn move_selection(&self, down: bool) -> Option<RephraseView> {
        let mut inner = self.lock();
        let pending = inner.pending.as_mut()?;
        pending.view.selected = if down && pending.native_selectable() {
            Row::Native
        } else if down {
            pending.view.selected
        } else {
            Row::Original
        };
        Some(pending.view.clone())
    }

    /// Close the pending choice and return the selected text.
    pub fn take_selected(&self) -> Option<String> {
        let mut inner = self.lock();
        let pending = inner.pending.take()?;
        match (pending.view.selected, pending.view.native) {
            (Row::Native, Some(native)) if !pending.view.already_natural => Some(native),
            _ => Some(pending.view.original),
        }
    }

    /// Close the pending choice without returning text. Returns whether one
    /// was pending.
    pub fn dismiss(&self) -> bool {
        self.lock().pending.take().is_some()
    }

    pub fn is_pending(&self) -> bool {
        self.lock().pending.is_some()
    }

    fn is_pending_id(&self, id: u64) -> bool {
        self.lock().pending.as_ref().is_some_and(|p| p.id == id)
    }

    fn set_swallow_release(&self, binding_id: &str) {
        self.lock().swallow_release_for = Some(binding_id.to_string());
    }

    /// Returns true (and clears the flag) if this release should be ignored.
    fn take_swallow_release(&self, binding_id: &str) -> bool {
        let mut inner = self.lock();
        if inner.swallow_release_for.as_deref() == Some(binding_id) {
            inner.swallow_release_for = None;
            true
        } else {
            false
        }
    }
}

/// Trim whitespace, surrounding quotes, and a leading "Rewritten:"-style label
/// from a model reply.
fn clean_output(raw: &str) -> String {
    let text = strip_invisible_chars(strip_think_block(raw));
    let mut text = text.trim();
    for quote in [('"', '"'), ('“', '”'), ('\'', '\'')] {
        if text.len() >= 2 && text.starts_with(quote.0) && text.ends_with(quote.1) {
            text = text[quote.0.len_utf8()..text.len() - quote.1.len_utf8()].trim();
        }
    }
    text.to_string()
}

/// True when the rewrite differs from the original only in case, whitespace,
/// or trailing punctuation.
fn is_already_natural(original: &str, native: &str) -> bool {
    fn normalize(s: &str) -> String {
        s.trim()
            .trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }
    normalize(original) == normalize(native)
}

/// Ask the active post-processing provider for a native-speaker rewrite.
async fn request_rephrase(settings: &AppSettings, text: &str) -> Result<String, String> {
    let provider = settings
        .active_post_process_provider()
        .cloned()
        .ok_or("no AI provider selected in Post Process settings")?;
    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    if model.trim().is_empty() {
        return Err(format!("no model selected for {}", provider.label));
    }

    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            if !crate::apple_intelligence::check_apple_intelligence_availability() {
                return Err("Apple Intelligence is not available".into());
            }
            let token_limit = model.trim().parse::<i32>().unwrap_or(0);
            return crate::apple_intelligence::process_text_with_system_prompt(
                SYSTEM_PROMPT,
                text,
                token_limit,
            )
            .map(|s| clean_output(&s))
            .map_err(|e| e.to_string());
        }
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        return Err("Apple Intelligence is not supported on this platform".into());
    }

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    let disable_reasoning = matches!(provider.id.as_str(), "custom" | "openrouter");

    if crate::llm_client::is_bedrock_converse_model(&provider, model.trim()) {
        let reply = crate::llm_client::send_bedrock_converse(
            &provider,
            api_key,
            model.trim(),
            text.to_string(),
            Some(SYSTEM_PROMPT.to_string()),
        )
        .await?
        .ok_or("empty response")?;
        let cleaned = clean_output(&reply);
        return if cleaned.is_empty() {
            Err("empty response".into())
        } else {
            Ok(cleaned)
        };
    }

    let reply = crate::llm_client::send_chat_completion_with_schema(
        &provider,
        api_key,
        &model,
        text.to_string(),
        Some(SYSTEM_PROMPT.to_string()),
        None,
        disable_reasoning,
    )
    .await?
    .ok_or("empty response")?;

    let cleaned = clean_output(&reply);
    if cleaned.is_empty() {
        Err("empty response".into())
    } else {
        Ok(cleaned)
    }
}

fn emit_view(app: &AppHandle, view: &RephraseView) {
    let _ = app.emit_to(utils::REPHRASE_PANEL_LABEL, "rephrase-state", view);
}

fn panel_bindings(app: &AppHandle) -> Vec<ShortcutBinding> {
    let dismiss_key = get_settings(app)
        .bindings
        .get("cancel")
        .map(|b| b.current_binding.clone())
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| "escape".to_string());
    [
        (REPHRASE_UP_BINDING, "up".to_string()),
        (REPHRASE_DOWN_BINDING, "down".to_string()),
        (REPHRASE_DISMISS_BINDING, dismiss_key),
    ]
    .into_iter()
    .map(|(id, key)| ShortcutBinding {
        id: id.to_string(),
        name: id.to_string(),
        description: String::new(),
        default_binding: key.clone(),
        current_binding: key,
    })
    .collect()
}

/// Register ↑/↓/Esc if the choice `id` is still pending. Runs off the hotkey
/// thread: handy-keys registration blocks on that thread's reply.
fn register_panel_keys(app: &AppHandle, id: u64) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<RephraseState>>();
        let mut registered = state
            .keys_registered
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if *registered || !state.is_pending_id(id) {
            return;
        }
        for binding in panel_bindings(&app) {
            if let Err(e) = shortcut::register_shortcut(&app, binding.clone()) {
                warn!("Failed to register rephrase key '{}': {}", binding.id, e);
            }
        }
        *registered = true;
    });
}

fn unregister_panel_keys(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Arc<RephraseState>>();
        let mut registered = state
            .keys_registered
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !*registered {
            return;
        }
        for binding in panel_bindings(&app) {
            let _ = shortcut::unregister_shortcut(&app, binding);
        }
        *registered = false;
    });
}

/// Show the panel for `text` instead of pasting it, and start the rewrite.
pub fn begin(app: &AppHandle, text: String) {
    let state = Arc::clone(&app.state::<Arc<RephraseState>>());
    let (id, view) = state.open(text.clone());
    utils::show_rephrase_panel(app);
    emit_view(app, &view);
    register_panel_keys(app, id);

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let settings = get_settings(&app);
        let result = match tokio::time::timeout(
            REPHRASE_TIMEOUT,
            request_rephrase(&settings, &text),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err("timed out".to_string()),
        };
        if let Err(e) = &result {
            warn!("Native rephrase failed: {}", e);
        }
        if let Some(view) = state.set_result(id, result) {
            emit_view(&app, &view);
        }
    });
}

fn close(app: &AppHandle) {
    utils::hide_rephrase_panel(app);
    unregister_panel_keys(app);
}

fn commit(app: &AppHandle, binding_id: &str) {
    let state = app.state::<Arc<RephraseState>>();
    let Some(text) = state.take_selected() else {
        return;
    };
    state.set_swallow_release(binding_id);
    close(app);

    let app_for_paste = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = utils::paste(text, app_for_paste.clone()) {
            error!("Failed to paste rephrase choice: {}", e);
            let _ = app_for_paste.emit("paste-error", ());
        }
    });
}

/// Route a shortcut event to the panel. Returns true when the event was
/// consumed and must not reach the normal shortcut handling.
pub fn handle_shortcut(app: &AppHandle, binding_id: &str, is_pressed: bool) -> bool {
    let Some(state) = app.try_state::<Arc<RephraseState>>() else {
        return false;
    };

    if !is_pressed && is_transcribe_binding(binding_id) && state.take_swallow_release(binding_id) {
        return true;
    }

    let is_panel_key = matches!(
        binding_id,
        REPHRASE_UP_BINDING | REPHRASE_DOWN_BINDING | REPHRASE_DISMISS_BINDING
    );
    if !state.is_pending() {
        // A panel key can still fire in the moment before it is unregistered.
        return is_panel_key;
    }

    if is_transcribe_binding(binding_id) {
        if is_pressed {
            debug!("Rephrase panel: pasting selected choice");
            commit(app, binding_id);
        }
        return true;
    }

    if !is_pressed {
        return is_panel_key;
    }

    match binding_id {
        REPHRASE_UP_BINDING | REPHRASE_DOWN_BINDING => {
            if let Some(view) = state.move_selection(binding_id == REPHRASE_DOWN_BINDING) {
                emit_view(app, &view);
            }
            true
        }
        REPHRASE_DISMISS_BINDING => {
            debug!("Rephrase panel dismissed");
            state.dismiss();
            close(app);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_loading_with_original_selected() {
        let state = RephraseState::default();
        let (_, view) = state.open("hello".into());
        assert!(view.loading);
        assert_eq!(view.selected, Row::Original);
        assert!(state.is_pending());
    }

    #[test]
    fn success_selects_native_and_pastes_it() {
        let state = RephraseState::default();
        let (id, _) = state.open("I want go shop".into());
        let view = state
            .set_result(id, Ok("I want to go shopping.".into()))
            .unwrap();
        assert_eq!(view.selected, Row::Native);
        assert!(!view.loading);
        assert_eq!(
            state.take_selected().as_deref(),
            Some("I want to go shopping.")
        );
        assert!(!state.is_pending());
    }

    #[test]
    fn arrows_move_between_rows() {
        let state = RephraseState::default();
        let (id, _) = state.open("a b".into());
        state.set_result(id, Ok("c d".into()));
        assert_eq!(state.move_selection(false).unwrap().selected, Row::Original);
        assert_eq!(state.move_selection(false).unwrap().selected, Row::Original);
        assert_eq!(state.move_selection(true).unwrap().selected, Row::Native);
        assert_eq!(state.move_selection(true).unwrap().selected, Row::Native);
        state.move_selection(false);
        assert_eq!(state.take_selected().as_deref(), Some("a b"));
    }

    #[test]
    fn down_is_ignored_while_loading_or_on_error() {
        let state = RephraseState::default();
        let (id, _) = state.open("orig".into());
        assert_eq!(state.move_selection(true).unwrap().selected, Row::Original);
        let view = state.set_result(id, Err("boom".into())).unwrap();
        assert_eq!(view.error.as_deref(), Some("boom"));
        assert_eq!(state.move_selection(true).unwrap().selected, Row::Original);
        assert_eq!(state.take_selected().as_deref(), Some("orig"));
    }

    #[test]
    fn identical_rewrite_is_already_natural() {
        let state = RephraseState::default();
        let (id, _) = state.open("See you tomorrow".into());
        let view = state
            .set_result(id, Ok("See you tomorrow.".into()))
            .unwrap();
        assert!(view.already_natural);
        assert_eq!(view.selected, Row::Original);
        assert_eq!(state.move_selection(true).unwrap().selected, Row::Original);
        assert_eq!(state.take_selected().as_deref(), Some("See you tomorrow"));
    }

    #[test]
    fn stale_result_is_ignored() {
        let state = RephraseState::default();
        let (old, _) = state.open("first".into());
        state.dismiss();
        let (_, _) = state.open("second".into());
        assert!(state.set_result(old, Ok("x".into())).is_none());
    }

    #[test]
    fn release_swallowed_once() {
        let state = RephraseState::default();
        state.set_swallow_release("transcribe");
        assert!(!state.take_swallow_release("other"));
        assert!(state.take_swallow_release("transcribe"));
        assert!(!state.take_swallow_release("transcribe"));
    }

    #[test]
    fn clean_output_strips_quotes_and_think() {
        assert_eq!(clean_output("  \"Hello there.\"  "), "Hello there.");
        assert_eq!(clean_output("“Hi.”"), "Hi.");
        assert_eq!(clean_output("<think>hmm</think> Hi."), "Hi.");
        assert_eq!(clean_output("It's fine."), "It's fine.");
    }
}
