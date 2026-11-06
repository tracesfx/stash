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
Post-v1 goals (SPEC section 11): local voice dictation with a user-supplied model (not bundled) and UI polish — do not build yet, but do not block them.
Open perf item: idle RSS was ~66 MB in a debug build after a popup (target <40 MB) — measure in release; if still high, avoid keeping WebView2 alive after popup close.
Test by simulating input: PowerShell scripts using keybd_event/mouse_event (see session scratchpad pattern); build with npx tauri build --debug --no-bundle (plain cargo build uses devUrl).
Product decisions: ONE global hotkey `Ctrl+Alt+S` — tap = instant region snip, hold ≥200 ms = radial wheel (SPEC §4). Post-capture popup is clickable for a caption. Metadata schema in SPEC §5.
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
