# Stash — project guide for Claude

Windows-only, ultra-fast inspiration capture tray app. Requirements live in `SPEC.md` (source of truth; update it when decisions change).

## Non-negotiables
- Capture = hotkey + drag, saved with no further input. Never add a required dialog, prompt, or naming step.
- Idle footprint is a feature: no polling, no replay buffer, no windows open when idle. Create webview windows on demand, destroy on close.
- One shared library folder of plain files + JSON sidecars (SPEC §5). Never require a DB to read the library.
- UI is never on the critical save path; saving happens in Rust.
- Feedback is a non-focus-stealing toast. Never steal focus from the user's app.
- No network calls, accounts, or telemetry.

## Stack & layout (planned)
Tauri v2 · Rust core in `src-tauri/` · web UI in `src/` (TypeScript, minimal deps).
Rust modules: `capture/` (screen, clipboard, selection), `library/` (ids, sidecars, index, trash), `hotkeys.rs`, `tray.rs`, `overlay/`, `config.rs`.

## Status
M0 + M1 done: native Win32 snip overlay (overlay.rs, decision gate -> native; webview too slow to cold-start), frozen frame at key-down, crop + PNG + metadata sidecar (library.rs), webview confirmation popup with click-to-caption (popup.rs, popup.html/ts). M1.5 done: hold >= hold_ms opens the native wheel (wheel.rs); snip.rs owns the Idle/Down/Busy state machine. Wheel wedges C2-C8 other than Region/Pause are dimmed stubs until M2/M3.
M2 done: clip.rs (selection/clipboard), note.rs + note.html (quick note), uia.rs (browser URL via UI Automation, config capture_url), library.rs now saves image/text/link/file. Popup/note webviews are built on background threads (first WebView2 start ~1s). Heavy save work runs off the state machine.
Testing caution: simulated-input test scripts act on whatever window is foreground (the user may be gaming) and Ctrl+C in a console kills the script — prefer a dedicated Notepad and check the foreground first, or ask the user to try by hand.
M3 done: gallery.rs (scan sidecars, stash:// protocol for thumbs, meta/trash/restore/import commands), gallery.html + src/gallery.ts (UI; user text is always set via textContent). Opens from tray left-click, wheel Gallery wedge, or a second app launch. Drag-out uses tauri-plugin-drag. NEVER run blind click/key simulation: it hit the users YouTube tab once.
M4 done: settings.rs/settings.html/src/settings.ts, runtime-mutable config (AppState.cfg()), autostart plugin (only in release; running the release exe from target/ registers HKCU Run entry Stash - delete it after tests), NSIS installer at src-tauri/target/release/bundle/nsis, portable = src-tauri/target/release/stash.exe. Release build takes ~6 min (LTO).
UI polish P1+P2 done (SPEC section 11 plan): src/theme.css tokens (accent = CSS AccentColor, dark/light via system), icons from icons-src/*.svg via scripts/make-icons.mjs + npx tauri icon, tray icon states, gallery rail/viewer, settings cards, Mica on gallery/settings. Next: P3 popup+note restyle, P4 wheel+overlay (Direct2D). Launch the exe with --settings while it runs to open settings (testing aid). Windows launched in the background open behind other apps; for screenshots raise only Stash windows with SetWindowPos TOPMOST.
UI polish P3 + themes + new wheel done: theme.rs (resolve theme, accent, window backdrops), src/theme.ts + theme.css (System/Light/Dark/Glass), wheel.rs now tiny-skia + fontdue (system Segoe UI) via UpdateLayeredWindow; FrameCell in snip.rs shares the frozen frame with the wheel. Config files from older versions lack newer fields (serde default fills them) so test scripts must Add-Member, not assign. Remaining polish: snip overlay (P4b).
Wheel is now 9 wedges (N/STEP consts in wheel.rs; Settings wedge added before Pause); slice gaps are constant-width via offset edges. theme::Style {theme, accent, glass[3]} is passed to wheel + overlay; web reads theme_info. gfx.rs has shared text drawing. Snip overlay: GDI base + tiny-skia pill. Test tip: verify the overlay opened via screen-brightness drop before sending clicks; set hold_ms=400 in a test config so simulated taps stay taps.
M5 done: palette.rs (histogram palette), ocr.rs (Windows.Media.Ocr via windows crate, .join() for async), library::update_sidecar (global lock; use it for ALL sidecar read-modify-writes), gallery.rs backfill/boards/export commands, gallery.ts colour/boards/shuffle/export. Sidecar fields added: palette[], ocr, board. Voice dictation is the next planned item.
Post-v1 goals (SPEC section 11): local voice dictation with a user-supplied model (not bundled) and UI polish — do not build yet, but do not block them.
Open perf item: idle RSS was ~66 MB in a debug build after a popup (target <40 MB). M6 removed the per-capture WebView2 (radial/note/toast are native), so re-measure idle RSS in release after captures.
Test by simulating input: PowerShell scripts using keybd_event/mouse_event (see session scratchpad pattern); build with npx tauri build --debug --no-bundle (plain cargo build uses devUrl).
Product decisions: ONE global hotkey `Ctrl+Alt+S` — tap = instant region snip, hold ≥200 ms = radial wheel (SPEC §4). After a capture the native tag radial appears (M6; the webview popup is gone). Metadata schema in SPEC §5.
Tooling gotchas: apostrophes inside Bash heredocs break in this harness — create files with the Write tool. Log files are `%APPDATA%\Stash\logs\stash.log.<date>`.

## Commands
- Dev: `npm run tauri dev`
- Build: `npm run tauri build`
- Checks: `cargo clippy --manifest-path src-tauri/Cargo.toml` · `cargo test --manifest-path src-tauri/Cargo.toml`
- Windows host: PowerShell or Git Bash; forward slashes in Bash.

## Conventions
- Rust: `anyhow` for app errors, `tracing` logs to `%APPDATA%\Stash\logs`. No `unwrap()` on I/O paths.
- Atomic writes: temp file + rename; image first, sidecar last.
- Capture code must be correct on multi-monitor and mixed per-monitor DPI.
- When touching capture/overlay, measure against SPEC §9 and report the numbers.
- Keep dependencies small; justify each new crate in the commit message.
- Go milestone by milestone (SPEC §10); don't build M3+ features during M1.

## Environment (checked 2026-11-05)
node 24, npm 11, cargo/rustc 1.99, .NET 9, git 2.56. Python not installed.

## Update (video / whiteboard / customization)
- Wheel now has 5 wedges: Region, Note, Record, Gallery, Settings (window/screen/selection/clipboard/pause/board removed from the wheel; Pause + New whiteboard live in the tray; clip.rs deleted).
- record.rs: Media Foundation H.264/AAC MP4 from GDI BitBlt, one thread only while recording; hotkey tap stops it. `cargo test record_selftest -- --ignored` smoke-tests it.
- board.rs + src/board.ts: whiteboard (canvas, scene = `<id>.board` next to the PNG). Videos on boards reference library files via `stash://media/<rel>`. Gallery selection -> New whiteboard (`gallery_new_board`).
- Config `appearance` (config.rs `Appearance`) drives web CSS vars (theme.ts) and the native wheel/overlay (theme::Style). Edits apply live via `settings_set_appearance`.
- Gallery: marquee select, card checkboxes, floating selection bar.

## Update (configurable wheel + paste/drop)
- Wheel wedges are now user-toggleable (Settings > Radial wheel, config `wheel_actions`, live via `settings_set_wheel`). 11 possible actions in fixed canonical order (wheel.rs `ALL_ACTIONS`): Region, Window, Screen, Selection, Clipboard, Note, Record, Whiteboard, Pause, Gallery, Settings; default = Region/Note/Record/Gallery/Settings; min 2. Wedge count is dynamic (`State.acts`). clip.rs is back (selection/clipboard + `clip::save`); Pending carries the foreground `window_rect` for Window capture.
- Gallery adds content by Ctrl+V (`gallery_paste`, reads native clipboard) and by HTML5 drop (native drag-drop handler disabled on the gallery window): files -> `gallery_import_data` (raw bytes, `x-name` header), text/URLs -> `gallery_import_text`. Untested by simulated input per the testing caution; try by hand.

## Update (M6: capture & look) — supersedes older notes above where they conflict
- Liquid Glass is the only theme. theme.rs has no Resolved/theme field; config `theme`/`show_popup`/`popup_ms` are gone (unknown fields are ignored on load). `Config::migrate` (style_rev < 2) resets the glass knobs/colours to the neutral M6 defaults and drops the `board` wheel action. Look references: reference/liquid-glass (gitignored; user picks 01 07 10 15 23 39 42 43 51 53 55).
- Native feedback windows (no WebView2 on the capture path): radial.rs (post-capture tag radial), note.rs (quick note, `#words` -> tags), toast.rs (notices + confirmation when the tag menu is off). Shared: gfx.rs (fonts, glass_fill/glass_rim material, blur_backdrop, `Layered` per-pixel-alpha window), textedit.rs (single-line field with the SPEC §4 editing keys, unit-tested), tags.rs (tag usage index built in the background from sidecars, `top`/`bump`/`hashtags`). popup.rs/popup.html/note.html are deleted; `library::Preview::Image` now carries a raw RGBA `Thumb`.
- Whiteboard removed entirely (board.rs, board.html, src/board.ts, tray item, wheel action, gallery_new_board/edit_board, stash://media). The sidecar `board` field is legacy (gallery still shows it until M7 folds it into tags).
- Settings autosave (no Save/Cancel): the page calls `settings_save` with the whole config on every change (debounced); `apply` only re-registers the hotkey / autostart / tag index / window look when those changed. Esc calls `settings_close`. Appearance is a hidden tab (footer button, remembered in localStorage).
- Gallery and settings windows are created `visible(false)`; the page calls `window_ready` (gallery.rs) right after layout (NOT from requestAnimationFrame: a hidden page gets no frames), with a 1.5 s safety net (`show_if_not_ready`); the entrance (body.pre -> body.opening) starts on the first visible frame. Gallery cards stagger by layout position (`stagger()`, --d) so masonry rows enter together; rail/type/date/colour switches use `switchTo()` (dip out, cascade in). Cards built during a cascade get no `.pop` class, otherwise removing the cascade class replays an animation. Settings closes on window blur unless `modal` (folder/colour pickers) is set.
- Look knobs: window glass (`win_blur/win_translucency/win_saturation`) and native glass (`glass_blur/glass_translucency/glass_saturation`) are separate; `glass_tint` (sheen), hover colour, animations are shared. Blur 0 = clear; frost fades in with blur (`theme::frost_of`, mirrored in theme.ts). `glass_clear` is legacy (read only by `Config::migrate`, style_rev 3). Native glass is previewed via `appearance_preview` (PNG over `PreviewBg` desktop snapshot).
- backdrop.rs: gallery/settings backdrop = Windows.UI.Composition visual (DesktopWindowTarget, not topmost) with the DWM host backdrop through our ColorMatrix (+GaussianBlur above HOST_BLUR) effect; `#[implement]` IGraphicsEffect objects; main thread only (`theme::restyle_window` queues it); Acrylic fallback. `tauri set_effects(None)` resets DWMWA_USE_HOSTBACKDROPBRUSH, so it must run BEFORE `backdrop::apply` (black backdrop otherwise). Debug builds: `STASH_BACKDROP_DEBUG=color|raw` paints a red test fill / the unprocessed host backdrop. Screenshot the backdrop on the gallery of a devtest instance (settings closes when it loses focus). Note position lives in ui_state.json (`note::note_reset_position`), never in config.json, so settings autosave cannot clobber it; the note window uses WDA_EXCLUDEFROMCAPTURE so its backdrop can be re-grabbed after a drag. Preview env overrides: STASH_PREVIEW_TR/TINT/BLUR/SAT. Web font sizes are rem (html font-size = --fs). Interactive highlights use --hover.
- Tag radial has no timer: closes when the pointer leaves (OFF_GRACE_MS), 12 s idle safety net; config `tag_menu_ms` was removed. Preview env overrides: STASH_PREVIEW_TR / STASH_PREVIEW_TINT.
- `winutil::bring_to_front` (AttachThreadInput + SetForegroundWindow, TOPMOST fallback) is used for gallery/settings/note/radial; `restore_focus` gives focus back to `SourceInfo.hwnd` (not serialized).
- Frame grab is ~30 ms; the old slow log numbers included wheel time (see SPEC §8). Benchmark: `cargo test grab_bench -- --ignored --nocapture`.
- Testing without touching the user's session: (1) native windows render offscreen to PNG via hidden windows: `STASH_PREVIEW_BG=<image> STASH_PREVIEW_OUT=<dir> cargo test previews -- --ignored`. (2) Web pages: `npm run build`, serve dist with a mocked `window.__TAURI_INTERNALS__` and screenshot with `msedge --headless=new --screenshot` (pattern in the M6 session scratchpad: mockserve.cjs). (3) A live debug instance next to the user's running Stash needs a different identifier (`npx tauri build --debug --no-bundle --config <json with identifier com.stash.devtest>`) AND a separate APPDATA, or it shares config/library and single-instance. Debug builds accept `--demo radial|textnote|note|toast` via a second launch; radial/textnote write a real sample item into the configured library.

## Update (M7 gallery + look fixes)
- gallery.ts is rewritten: tabs Notes/Visuals/All (localStorage `stash.gallery.tab`), day sections (`.day` > sticky `.dh` + `.dbody` of absolutely positioned cards), JS masonry in `layout(anim)` with FLIP via Web Animations ("none" | "flip" | "switch" | "enter"), cards cached per kind (`card:`/`note:` + id) so images stay decoded. Scrubber (#scrub), filter chips (#chips), tag collections (rail tiles), chip tag input with autocomplete (`tagInput`), selection bar "Tag…" (`gallery_tag`), arrow-key selection, viewer zooms from its card.
- Canvas view: per-scope layout in `<library>/.views/<scope>.json` (`gallery_view_load/save`), auto-arranged by source page, Save as image renders a PNG and imports it via `gallery_import_data`.
- library.rs `set_notifier/notify_changed` (put, set_caption, add_tags) -> lib.rs emits `library-changed` to the gallery. Background palette/OCR updates do not notify.
- Headless Edge `--screenshot` with virtual time does NOT advance Web Animations: to check motion, drive Edge over CDP in real time (session scratchpad mock/cdp.cjs pattern).
- theme.rs disables DWM transitions on gallery/settings (debug: STASH_DWM_TRANSITIONS=1 restores them for A/B). Note drag re-grabs its backdrop per move (throttled ~14 ms). `Appearance.cursor_glow`, `Config.settings_autoclose`. Presets were removed from Settings.
