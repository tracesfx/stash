# Stash — Inspiration Capture Spec (v0.2)

Working name: **Stash**. Windows 11 only. Mobile is out of scope.

## 1. Problem
Inspiration (an image, video frame, quote, page) is seen for seconds. The current path —
Snipping Tool → Ctrl+V into Notion/whiteboard → tag/organise — has too much friction,
so things never get saved. Capture must cost **one hotkey + one drag** and nothing else.

## 2. Principles (in priority order)
1. **Capture speed over everything.** Tap hotkey + drag → saved on disk <300 ms after release. No dialogs, no naming, no required fields.
2. **Invisible when idle.** Tray only. No replay buffer, no screen recording, no polling. Idle target: <40 MB RAM, ~0% CPU, no windows open.
3. **One library.** Every capture, from every method, lands in a single folder. Organisation happens later, on the user's terms.
4. **Plain files, no lock-in.** Library = normal files + JSON sidecars. Usable without the app; syncable with OneDrive/Dropbox/git.
5. **Unobtrusive feedback.** A brief corner popup; never steals focus on its own; never a modal.

## 3. Non-goals
Mobile, cloud accounts/sync service, collaboration, always-on recording/replay buffer, OCR/AI tagging (later, optional), video clip recording (v1 saves frames only), a full whiteboard (export to one instead).

## 4. Capture methods (all write to the same library)
**One global hotkey: `Ctrl+Alt+S`** (rebindable). No other global hotkeys.
- **Tap** (release < 200 ms, or start dragging): instant **region snip** (C1).
- **Hold** (≥ 200 ms): a **radial wheel** appears centred on the cursor; move toward a wedge and release the key (or click) to choose; release in the dead centre or `Esc` cancels. Same flick always picks the same wedge (fixed wedge positions).
- The wheel window is topmost but **no-activate** (never takes focus), so the foreground app keeps its selection/clipboard state for C4.
- Needs key-down **and** key-up events for the hotkey (Tauri global-shortcut provides both).

| Wedge | Method | Behaviour |
|----|--------|-----------|
| C1 | **Region snip** (also the tap default) | Screen frozen at keypress (catches video frames / hover states). Dim overlay, drag a rect, release = saved as PNG. `Esc` cancels. **No clipboard step.** Multi-monitor + per-monitor DPI correct. |
| (removed from wheel) C2 | **Active window** | Instant save, no overlay. |
| C3 | **Full screen** (current monitor) | Instant save. |
| C4 | **Stash selection** | App sends Ctrl+C to the foreground app, reads the clipboard (text / image / URL / files), saves it, restores the prior clipboard. |
| C5 | **Stash clipboard** | Saves current clipboard contents as-is. |
| C6 | **Quick note** | Tiny single-line input at a screen corner (takes focus — the one intentional exception); Enter saves a text item, Esc discards. |
| C7 | **Open gallery** | Opens the gallery window. |
| C8 | **Settings** and **Pause** | Two wedges (the wheel has 9 slices of 40 degrees): open Settings; toggle capture pause. |
| — | **Drag & drop** | Drop files/images/links onto the gallery window (no hotkey). |

**Confirmation popup + caption:** after every capture a small thumbnail popup appears in a screen corner for ~1.5 s (pauses while hovered; never takes focus on its own). Clicking it opens an inline caption field (focus taken only on click); Enter saves the caption, Esc closes. Ignoring it = saved untouched. All text inputs (caption, quick note, search, settings) use native text controls so standard editing shortcuts work (Ctrl+Backspace/Delete word-delete, Ctrl+A/Z/arrows, Shift-select); this is an acceptance test for M1/M2.

Every item auto-records metadata (see §5): timestamp, source window title + process, source URL when the foreground app is a supported browser (UI Automation, best-effort), capture method, dimensions, monitor/scale, capture rect, file size/format, content hash.

## 5. Library
- Root folder (default `%USERPROFILE%\Pictures\Stash`, configurable, may be a synced folder). Layout: `YYYY-MM/<id>.<ext>` + `<id>.json` sidecar.
- `id` = `YYYYMMDD-HHMMSS-xxxx` (sortable, collision-safe across machines).
- Types: `image`, `text`, `link`, `file`.
- Sidecar schema v1 (`schema` field versioned; unknown fields preserved on rewrite): `{ schema, id, type, created, modified, method, caption, tags:[], source:{title,process,exe_path?,url?}, file:{name,ext,bytes,sha256,mime,original_path?}, image:{w,h,monitor,scale,rect}, text:{chars?}, palette:[], rating?, board? }`. Metadata is written at capture time; editing caption/tags updates `modified`.
- In-memory index built from sidecars at gallery open (folder watcher for external changes). No required database; an optional disposable SQLite cache may be added past ~20k items.
- Delete = move to `.trash/` inside the library (recoverable).

## 6. Gallery window (secondary surface)
Opened from tray click or wheel wedge C7; created on demand, destroyed on close.
- Masonry grid, newest first; lazy thumbnails cached in `.thumbs/`.
- Instant search over caption, tags, source title/URL, type; filter by tag/date/colour.
- Item actions: copy image, **drag out** into Notion/Figma/Miro/Explorer, open source URL, edit tags/caption, delete, reveal in Explorer.
- **Shuffle view** (P2): full-screen random item, one key for next.
- Multi-select → export/copy to folder.

## 7. Tray & settings
Tray menu: Open gallery · Pause capture · Open folder · Settings · Quit.
Settings: library path, hotkey rebinding, wheel hold delay, start with Windows (default on), popup/sound toggles, image format (PNG/WebP), include cursor. Config: `%APPDATA%\Stash\config.json`.

## 8. Technical approach
- **Stack:** Tauri v2 (Rust core + small TypeScript web UI, no heavy framework). Native hotkeys/tray/capture in Rust; WebView2 windows exist only when needed.
- **Crate candidates:** `tauri-plugin-global-shortcut`, `xcap`, `arboard`, `windows` (foreground window, UIA, DPI), `notify`, `image`, `sha2`, `serde_json`, `tauri-plugin-autostart`, `tauri-plugin-single-instance`, `anyhow`, `tracing`.
- **Overlay:** frozen screenshot in a borderless topmost per-monitor window, spawned at hotkey, closed on finish. Must appear <150 ms; if WebView2 cold start misses this, fall back to a native Win32 layered-window overlay (**decision resolved in M1: snip overlay is native Win32**; popup/gallery stay webview because native edit controls lack Ctrl+Backspace). Same gate applies to the wheel and popup.
- **Popup/wheel:** topmost, no-activate (`WS_EX_NOACTIVATE`) windows.
- Capture pipeline runs fully in Rust; UI is never on the critical save path.
- Single instance; crash-safe writes (temp + rename; image first, sidecar last).

## 9. Targets
| Metric | Target |
|---|---|
| Hotkey tap → overlay visible | < 150 ms |
| Hold → wheel visible | < 100 ms after hold threshold |
| Mouse release → file on disk | < 300 ms (4K region) |
| Idle RAM / CPU | < 40 MB / ~0% |
| Installed size | < 25 MB |
| Gallery open, 10k items | < 1 s to first paint |
| Hotkey registration conflict | surfaced in tray, never silent |

## 10. Milestones
- **M0 Scaffold:** Tauri v2 project, tray, single-instance, config, library path, logging, hotkey press/release plumbing (log only).
- **M1 Snip core (the product):** C1 with frozen overlay, multi-monitor/DPI, save + sidecar + metadata, confirmation popup with click-to-caption. Overlay decision gate.
- **M1.5 Wheel (done):** hold-to-open native radial wheel, fixed wedge positions, release/click to choose, unavailable wedges dimmed until their milestone. Region + Pause live.
- **M2 More capture (done, partly verified):** C2–C6 (window, screen, selection, clipboard, note) + source window and browser URL metadata. Gallery wedge stays dimmed until M3.
- **M3 Gallery (built; grid verified, rest needs hands-on test):** masonry grid, lazy cached thumbnails, instant search (#tag, type:), type filter, caption/tag edit, multi-select, trash with undo, show-in-folder, open source URL, copy, native drag-out, file-drop import. Colour/date filters wait on palette extraction (M5).
- **M4 Polish (built):** settings window (rebind shortcut, hold delay, library folder, autostart, URL capture, popup toggle), autostart (release builds only), per-user NSIS installer (1.8 MB), portable single exe. Measured on release build: idle 21.8 MB RSS, 0 CPU, no webview process. Not built from the original settings list: sound, WebP output, include-cursor.
- **M5 (built; export and shuffle untested by hand):** dominant-colour palette (stored in the sidecar, colour filter + swatches in the gallery, `color:blue` search), boards (one board per item; rail + multi-select assign), shuffle view (S), export of a selection/view to a folder with files + index.md + notion.csv, and local OCR (Windows built-in engine) so screenshots are searchable by the text inside them; existing images are indexed in the background. Not built: short GIF/clip capture (conflicts with the no-recording principle), direct Notion/Miro API sync (no network by design).
- **Post-v1:** voice dictation (SPEC section 11), optional further polish.

## 11. Post-v1 goals
These are explicitly out of v1 scope; v1 must not make them harder.
1. **Voice dictation (local, optional).** Dictate captions and quick notes by voice, fully offline.
   - Speech model is **not bundled** with the app (keeps installer <25 MB). User downloads a model once (e.g. a Whisper-family GGML/ONNX model) and points Settings at its file; the app never downloads it silently and makes no network calls.
   - Runs in-process via an optional engine (candidate: whisper.cpp through the `whisper-rs` crate) behind a Cargo feature, or as an external local executable the user configures. The model is loaded only while dictating and unloaded afterwards, so idle RAM stays within §9.
   - UX: a mic button / push-to-talk key inside the caption field and quick-note input; text streams into the native input, so Ctrl+Backspace and all editing still work. Optional wheel wedge for "dictate a note".
   - Audio is processed in memory and never written to disk or the library.
2. **UI polish.** Anti-aliased wheel and overlay (per-pixel-alpha layered windows or Direct2D instead of colour-keyed GDI), open/close animations, popup transitions, refined typography/icons, light/dark themes, high-contrast and reduced-motion support, DPI/multi-monitor visual QA, app/tray icon design.

### UI polish plan (direction: Fluent glass, accent follows Windows)
Principles: dark default + light via system; Mica/translucent layers, soft borders, radii 8/12/16, 8 px grid, Segoe UI Variable; motion 120-180 ms ease-out, only after the save is done, respects reduced-motion; one token set (src/theme.css, mirrored as constants for native windows).
- **P1 Foundation (done):** theme.css tokens (accent = CSS AccentColor), app icon + tray states (normal / paused / hotkey-unavailable).
- **P2 Gallery + Settings (done; viewer/hover actions/filters untested by hand):** Mica window, slim top bar, pill search, left rail (tags, apps, date filters), hover actions, fade-in cards, arrow-key viewer, empty state; settings as grouped cards with toggles and keycap chips.
- **P3 Popup + Note (done):** slide-in, rounded card + shadow, countdown bar, metadata line, smooth caption expand.
- **P4 Wheel (done, 9 wedges: Settings added, constant-width parallel gaps) + Snip overlay (done: accent border, corner brackets, rounded size pill, glass-tinted dim):** Direct2D/DirectWrite per-pixel-alpha rendering (fallback: supersampled GDI); icon wedges, centre label, open/hover animation; accent border, corner handles, rounded size pill.
- **Themes (added):** System / Light / Dark / Liquid Glass, chosen in Settings (applies live). Web windows read data-theme from src/theme.ts; gallery/settings backdrop = Mica, or Acrylic for glass; the native wheel renders per-pixel-alpha with tiny-skia and, in glass, frosts a blurred copy of the frozen screen behind it. Liquid Glass gradient colours (3 stops) are configurable in Settings and drive the web windows, the wheel sheen, the snip pill and the Acrylic tint. One accent source (theme.rs): Windows accent, or brand blue if it is neutral grey.
- Guards: overlay/wheel still < 150 ms (measure each step); transparent webview windows fall back to solid cards if they flicker; Mica falls back to flat dark.

## 12. Open questions
1. Final name; is `Ctrl+Alt+S` free in the user's other tools?
2. Sync story: plain OneDrive/Dropbox folder (assumed) vs. none.
