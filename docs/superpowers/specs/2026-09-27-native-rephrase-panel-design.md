# Native Rephrase Panel — Design

Date: 2026-09-27
Status: Approved in conversation, pending spec review

## Goal

Help a non-native English speaker learn natural phrasing while dictating. After
each dictation, Handy shows the user's transcribed sentence next to a
native-speaker rewrite. The user picks one with the arrow keys and pastes it with
the same transcribe shortcut they already use.

## Non-goals

- Editing text inside the panel (the user edits in the target text field after pasting).
- Explanations or tips on what changed.
- A history/review log of corrections.
- Changing the existing workflow when the feature is off.

## User-facing behavior

New setting: **Native rephrase panel** (boolean, `native_rephrase_enabled`,
default `true`). When `false`, Handy behaves exactly as today.

When `true`, with the existing transcribe shortcut (currently Right ⌘ + O):

1. **Press 1** — starts recording (unchanged).
2. **Press 2 / release after hold** — stops recording and transcribes (unchanged).
   Instead of pasting, Handy requests a native rewrite from the active
   post-processing provider (the user configures AWS Bedrock (Mantle) with their
   API key and model in the existing Post-processing settings), then shows the
   rephrase panel:

   ```
     I want go to shop for buy milk
   ▶ I'm going to the store to grab some milk.
   ```

   The native rewrite is selected by default. While the request is in flight,
   the panel shows the original with a "Rephrasing…" placeholder in the second row.
3. **↑ / ↓** — move the selection between the two rows.
4. **Press 3** (transcribe shortcut) — pastes the selected text via the existing
   paste path, closes the panel. The key release after this press is ignored.
5. **Esc** (existing cancel binding) — closes the panel without pasting.

Applies to both `transcribe` and `transcribe_with_post_process` bindings; the
text sent for rephrasing is the pipeline's final text (after any existing
post-processing, filler removal, Chinese variant conversion).

If the transcription is empty, nothing is shown (unchanged behavior).

## Error handling

- Provider not configured, no API key, request error, or timeout (15 s): the
  panel shows only the original sentence with a short error line
  ("Rephrase unavailable: <reason>"). Press 3 pastes the original.
- If the rewrite comes back identical to the original (after trimming), show
  the original row only with the note "Already natural ✓".
- A new recording cannot start while the panel is open — press 3 always means
  "paste". Esc dismisses; after that the shortcut starts recording again.

## Architecture

### Key handling (no focus stealing)

The panel never becomes the key window, so the user's target app keeps focus
and the paste lands where they were typing. Arrow keys are captured with the
same mechanism as the dynamic `cancel` binding: `handy-keys`'
`HotkeyManager::new_with_blocking()` swallows registered hotkeys. While the
panel is open we register two dynamic bindings, `rephrase_up` (`up`) and
`rephrase_down` (`down`), plus `cancel` (`escape`); they are unregistered when
the panel closes. This means arrow keys do not reach the underlying app while
the panel is open — intended.

(If the user's `keyboard_implementation` is the Tauri global-shortcut backend,
registering bare arrow keys should also work; verify during implementation.)

### Units

| Unit | File(s) | Responsibility |
|---|---|---|
| Setting | `src-tauri/src/settings.rs`, settings UI toggle, i18n `en` string | `native_rephrase_enabled: bool`, default `true`, serde default for existing stores |
| Rephrase call | `src-tauri/src/rephrase.rs` (new) | `async fn rephrase(settings, text) -> Result<String, String>`: builds the fixed system prompt, calls the active provider via existing `llm_client::send_chat_completion` / Apple Intelligence path, strips think blocks/quotes, 15 s timeout |
| Review state | `src-tauri/src/rephrase.rs` | `RephraseState` (Tauri managed state, `Mutex<Option<PendingChoice>>`): `{ original, native: Option<String>, error: Option<String>, selected: Row, swallow_release: bool }`; methods `open`, `set_result`, `move_selection`, `take_selected`, `dismiss` |
| Panel window | `src-tauri/src/overlay.rs` | `create_rephrase_panel`, `show_rephrase_panel`, `hide_rephrase_panel`. macOS: NSPanel like the recording overlay (`can_become_key_window: false`, nonactivating, `no_activate`, floating/status level), positioned just above the recording overlay; other platforms: `WebviewWindowBuilder` with `focusable(false)`, always on top |
| Panel UI | `src/rephrase/index.html`, `main.tsx`, `RephrasePanel.tsx`, `RephrasePanel.css`; `vite.config.ts` new entry | Renders rows from a `rephrase-state` event; purely presentational, no input handling |
| Shortcut routing | `src-tauri/src/shortcut/handler.rs`, `handy_keys.rs` / `tauri_impl.rs` | In `handle_shortcut_event`: if `RephraseState` has a pending choice, (a) transcribe binding press → paste selected + close, release swallowed; (b) `rephrase_up/down` → move selection; (c) `cancel` press → dismiss. Otherwise, existing logic unchanged. Register/unregister helpers mirror `register_cancel_shortcut` |
| Pipeline hook | `src-tauri/src/actions.rs` | At the point where `utils::paste(final_text, …)` is called: if `native_rephrase_enabled`, call `RephraseState::open(final_text)`, show panel, hide recording overlay, spawn `rephrase` and push result; do **not** paste. Paste on press 3 reuses `utils::paste` on the main thread |

The transcription coordinator state machine is **not** modified: processing
finishes normally (coordinator returns to Idle), and the pending-choice check
in `handle_shortcut_event` intercepts the next press before it reaches the
coordinator.

### Rephrase prompt (fixed, not user-editable in v1)

System:
> You rewrite English sentences so they sound like a fluent native speaker said
> them. Keep the original meaning, tone, and level of formality. Fix grammar,
> word choice, and unnatural phrasing. Do not add new information. Output only
> the rewritten text, with no quotes, labels, or explanations.

User: the transcript.

### Data flow

```
press 2 → coordinator → actions pipeline → final_text
  └─ enabled? ── no → utils::paste (unchanged)
               └ yes → RephraseState.open(final_text)
                        → emit rephrase-state, show panel, register ↑/↓/esc
                        → spawn rephrase(final_text) → set_result → emit
↑/↓   → handler → move_selection → emit
press 3 → handler → take_selected → hide panel, unregister keys → utils::paste
esc   → handler → dismiss → hide panel, unregister keys
```

## Testing

- Rust unit tests in `rephrase.rs`: selection movement (clamped to available
  rows; only original row when native missing), `take_selected` returns the
  right text, identical-rewrite detection, output cleanup (quotes/think blocks).
- `cargo test` and `bun run lint` / `bun run format:check` pass.
- Manual on macOS: dictate into Notes, Chrome, Slack; confirm arrows don't move
  the cursor in the app while the panel is open, paste lands in the focused
  field, Esc dismisses, feature off = old behavior, Bedrock error path shows
  original only.
