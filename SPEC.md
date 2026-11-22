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
5. **Unobtrusive feedback.** After a capture, a native tag radial at the cursor that fades out by itself; notices are a small native toast. Never a modal, never a required step, and focus always returns to the app the user was in.

## 3. Non-goals
Mobile, cloud accounts/sync service, collaboration, always-on recording/replay buffer, AI tagging (embeddings deferred, see §11), a drawing whiteboard (removed in M6; the gallery's Canvas view arranges items instead).

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
| C6 | **Quick note** | Native single-line input; drag it anywhere and it reopens there (remembered in `%APPDATA%\Stash\ui_state.json`; until moved, or after Settings > Reset to corner, it uses the toast corner). It takes focus, the one intentional exception; focus returns to the previous app on close). Enter writes the text item and closes in the same frame; Esc discards. `#words` in the note become tags. |
| C7 | **Open gallery** | Opens the gallery window. |
| C8 | **Settings** and **Pause** | Open Settings; toggle capture pause. |
| — | **Drag & drop / paste** | Drop or Ctrl+V files/images/text/links into the gallery window (no hotkey). |

The wheel's wedges are user-configurable (Settings > Radial wheel; config `wheel_actions`, minimum 2). Possible actions, in fixed clockwise order: Region, Window, Screen, Selection, Clipboard, Note, Record, Pause, Gallery, Settings (the Whiteboard action is removed in M6; configs that still list `board` drop it on load).

**Tag radial (replaces the confirmation popup, M6):** the capture is already on disk before anything appears. Within ~50 ms of the mouse release, a small native radial opens at the release point: the capture's thumbnail in the centre (this is the confirmation), up to 6 tag wedges ranked by recency + frequency, and one Caption wedge.
- Click a wedge = add that tag and close; Ctrl+click = add and keep open for another tag.
- Typing filters the tags; Enter applies the match or creates a new tag. Tab (or the Caption wedge) switches the field to caption; Enter saves it.
- No timer and no countdown: it stays while the pointer is on it (ring, pill or the gap between) and fades out ~150 ms after the pointer moves off; the item stays saved untagged. While a tag or caption is being typed it stays regardless. Esc, right-click or clicking elsewhere closes it. Safety net: closes after 12 s with no pointer movement and nothing typed.
- Appears after Region, Window, Screen, Selection, Clipboard and finished recordings; not after notes (tagged inline). With no tags yet it shows just the input and the Caption wedge.
- Budget: tagging an item with an existing tag takes < 1.5 s, a new tag < 3 s.
- Writes go through `library::update_sidecar`. Errors and Pause messages use a small native toast.

**Text input:** the radial and the quick note use a native text field drawn with tiny-skia. It must support the standard editing keys: Ctrl+Backspace/Delete word-delete, Ctrl+A/C/X/V/Z, arrows, Ctrl+arrows, Home/End and Shift-select. This is an acceptance test for M6. Web windows (search, settings, gallery fields) keep native web inputs.

Every item auto-records metadata (see §5): timestamp, source window title + process, source URL when the foreground app is a supported browser (UI Automation, best-effort), capture method, dimensions, monitor/scale, capture rect, file size/format, content hash.

## 5. Library
- Root folder (default `%USERPROFILE%\Pictures\Stash`, configurable, may be a synced folder). Layout: `YYYY-MM/<id>.<ext>` + `<id>.json` sidecar.
- `id` = `YYYYMMDD-HHMMSS-xxxx` (sortable, collision-safe across machines).
- Types: `image`, `text`, `link`, `file`.
- Sidecar schema v1 (`schema` field versioned; unknown fields preserved on rewrite): `{ schema, id, type, created, modified, method, caption, tags:[], source:{title,process,exe_path?,url?}, file:{name,ext,bytes,sha256,mime,original_path?}, image:{w,h,monitor,scale,rect}, text:{chars?}, palette:[], ocr?, rating?, board? }`. Metadata is written at capture time; editing caption/tags updates `modified`.
- `board` is legacy (M5): the gallery reads a non-empty `board` as an extra tag and no longer writes it. Old `<id>.board` whiteboard scene files are left on disk untouched and ignored. Their PNG is a normal image item.
- Derived, disposable caches live in dot-folders (`.thumbs/`, `.views/` for Canvas layouts). Deleting them loses nothing but layout or speed.
- In-memory index built from sidecars at gallery open (folder watcher for external changes). No required database; an optional disposable SQLite cache may be added past ~20k items.
- Delete = move to `.trash/` inside the library (recoverable).

## 6. Gallery window (secondary surface)
Opened from tray click or wheel wedge C7; created on demand, destroyed on close.
- Lazy thumbnails cached in `.thumbs/`. Instant search over caption, tags, source title/URL, OCR text and type.
- Item actions: copy image, **drag out** into Notion/Figma/Miro/Explorer, open source URL, edit tags/caption, delete (with undo), reveal in Explorer, export.
- **Shuffle view:** full-screen random item, one key for next.

**Organisation (M7).** The library is mostly untagged notes, so navigation must work with no metadata:
- **Tabs:** Notes · Visuals · All. Notes is a readable list showing each note's source page title (e.g. the YouTube video). Visuals is a masonry grid of images, clips and files.
- **Timeline:** sticky day headers (Today, Yesterday, Mon 16 Nov…) and a date scrubber on the right edge to jump.
- **Collections:** tags shown as collections with cover thumbnails at the top of the rail (legacy `board` values fold in here). Active filters show as removable chips in the header.
- **Tagging in the gallery:** chip input with autocomplete; multi-select gets "Tag…" in the selection bar. Arrow keys move the selection.

**Motion (M7).** JS-positioned masonry (transform-based) instead of CSS columns, so changes animate in place instead of re-rendering the grid:
- Delete: the card shrinks out and neighbours glide into place. Undo grows it back where it was. Scroll position is kept.
- Filter and tab changes keep existing cards and animate only the difference.
- New captures arrive through a Rust `library-changed` event and slide into Today, instead of a reload on focus. The viewer zooms open from its card.
- 150-220 ms, transform/opacity only. Honours the Animations setting and Windows reduced motion.

**Canvas view (M7).** A Grid/Canvas toggle for the current scope (a day, a tag or any filter). Canvas auto-arranges the items: notes as sticky notes, images at their real aspect ratio, clustered by source page.
- Pan, zoom and drag to rearrange (no drawing tools). New items are placed in free space.
- Moved positions persist in `.views/<scope>.json`. Export to PNG saves into the library.

## 7. Tray & settings
Tray menu: Open gallery · Pause capture · Open folder · Settings · Quit (no "New whiteboard").
Settings (redesigned in M6):
- **Visible sections:** Capture (shortcut, hold delay), Radial wheel (wedges), Tag menu (on/off), note and toast corner, Recording (audio, fps, max length), Library (folder, OCR, URL capture), System (start with Windows).
- **Appearance:** glass colours and knobs are in a collapsed "Appearance" section, hidden by default.
- **Save model:** no Save or Cancel buttons. Every change is saved and applied as it is made (sliders debounced ~220 ms; the library folder only when its field is committed), confirmed by a small "Saved" tick in the footer. Esc closes the window.
- **Text size:** all web font sizes are rem-based, so the Text size knob scales every label, hint, keycap and count together.
- **Appearance groups:** Presets · Everywhere (glass colours, colour sheen, hover colour, animations) · Gallery and settings windows (blur, translucency, saturation, text size, corner roundness) · Wheel, tag menu and note (live preview, blur, translucency, saturation) · Wheel shape. Window and native glass have separate knobs (config `win_*` vs `glass_*`).
- **Blur:** 0 = clear (the sharp screen shows through) on both kinds of glass; there is no separate Transparent switch (style_rev 3 migrates it to blur 0). Translucency only thins the dark tint.
- **Window glass (backdrop.rs):** gallery and settings draw their own backdrop with Windows.UI.Composition: a visual behind the transparent web view filled with the DWM host backdrop, run through our saturation and extra blur. DWM delivers that backdrop already blurred (~30 px, the Acrylic look) and there is no API for the sharp content short of live screen capture, so blur 0-30 cross-fades the frosted layer over the sharp desktop and 30-60 stacks a real Gaussian blur on top. Changes apply live to the open window. Falls back to Acrylic if composition fails or Windows transparency effects are off. A true weak blur would need Windows.Graphics.Capture of the screen behind the window (not built).
- **Native glass** (wheel, tag menu, note, toasts) blurs a grab of the screen itself, so any blur radius is exact; the Appearance tab shows it live over a desktop snapshot taken when settings opens.
- **Window behaviour:** settings opens with an entrance animation and closes when it loses focus (not while its own folder/colour pickers are open). Gallery and settings are created hidden and shown by the page once laid out. Colour sheen (0-100 %) tints panels, the page and native glass with the three glass colours. The Hover and selection colour drives switches, focus rings, selections and the wheel highlight.
- **Layout:** custom-styled sliders with a value readout. Each row puts the label and description on top and the control in a fixed-width column (or below it on narrow rows), so descriptions never get squished. No version footer.
Config: `%APPDATA%\Stash\config.json`.

## 8. Technical approach
- **Stack:** Tauri v2 (Rust core + small TypeScript web UI, no heavy framework). Native hotkeys/tray/capture in Rust; WebView2 windows exist only when needed.
- **Crate candidates:** `tauri-plugin-global-shortcut`, `xcap`, `arboard`, `windows` (foreground window, UIA, DPI), `notify`, `image`, `sha2`, `serde_json`, `tauri-plugin-autostart`, `tauri-plugin-single-instance`, `anyhow`, `tracing`.
- **Overlay:** frozen screenshot in a borderless topmost per-monitor window, spawned at hotkey, closed on finish. Must appear <150 ms; if WebView2 cold start misses this, fall back to a native Win32 layered-window overlay (**decision resolved in M1: snip overlay is native Win32**). Same gate applies to the wheel.
- **M6 decision:** the post-capture popup and the quick note were webviews (0.3-1 s to appear). They become native per-pixel-alpha layered windows (tiny-skia + fontdue, like the wheel) with a custom text field (see §4). Gallery and settings stay webview.
- **Frame grab (measured in M6):** the GDI full-desktop grab takes ~30 ms (2560×1440; same with or without `CAPTUREBLT`, so the flag was dropped). The old "p90 1.7 s" log figure counted time spent in the wheel; tap-only snips open the overlay at median 101 ms / p90 152 ms after key-down. DXGI is not needed. The log now records the grab itself ("screen grab N ms").
- **Window focus:** opening an existing or new gallery/settings window must bring it to the front despite Windows' foreground lock (shared `winutil::bring_to_front`).
- **Wheel/radial/toast:** topmost layered windows. The wheel and toast are no-activate. The tag radial takes focus only to receive typing, and gives it back to the source window when it closes.
- Capture pipeline runs fully in Rust; UI is never on the critical save path.
- Single instance; crash-safe writes (temp + rename; image first, sidecar last).

## 9. Targets
| Metric | Target |
|---|---|
| Hotkey tap → overlay visible | < 150 ms (p90, not just median) |
| Hold → wheel visible | < 100 ms after hold threshold |
| Mouse release → file on disk | < 300 ms (4K region) |
| Mouse release → tag radial visible | < 80 ms |
| Note Enter → saved and window gone | < 50 ms |
| Idle RAM / CPU | < 40 MB / ~0% |
| Installed size | < 50 MB (currently ~2.4 MB) |
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
- **v0.2 (done):** wheel actions configurable, video clip recording (record.rs), whiteboards, paste/drop into the gallery.
- **M6 Capture & look (built; tag radial, note and settings need a hands-on pass):** all five items below. Native windows render offscreen for review (`cargo test previews -- --ignored`, see gfx::preview), and the settings page was checked in headless Edge with a mocked backend. Not yet measured on a live session: release → radial visible and note Enter → gone (both are logged: "tag radial visible N ms", "note saved in N ms").
  1. **Speed:** frame grab (§8); `bring_to_front` for gallery/settings; measure §9.
  2. **Native capture UI:** tag radial, native quick note with `#tags`, native toast. Remove `popup.rs`/`popup.html`/`popup.ts`; the "Confirmation popup" and popup position/duration settings become "Tag menu" settings.
  3. **Liquid Glass only:** remove System/Light/Dark. The `theme` config field is ignored and the app is always glass. Restyle the tokens (web) and the mirrored native constants (wheel, overlay, radial, toast) from the reference images the user picks in `reference/liquid-glass/` (gitignored). Fix the clipped shadow ("black rectangle") on corner cards.
  4. **Settings redesign** per §7.
  5. **Remove the old whiteboard:** `board.rs`, `board.html`, `src/board.ts`, tray item, wheel action, `gallery_new_board`/`gallery_edit_board`, `--board` arg, `scene` flag, and the `stash://media` route if nothing else uses it.
- **M7 Gallery:** organisation, motion and Canvas view per §6. Performance guard: 10k synthetic items < 1 s to first paint.
- **Post-v1:** voice dictation and embeddings (§11).

## 11. Post-v1 goals
These are explicitly out of v1 scope; v1 must not make them harder.
1. **Voice dictation (local, optional).** Dictate captions and quick notes by voice, fully offline.
   - Speech model is **not bundled** with the app (keeps installer <25 MB). User downloads a model once (e.g. a Whisper-family GGML/ONNX model) and points Settings at its file; the app never downloads it silently and makes no network calls.
   - Runs in-process via an optional engine (candidate: whisper.cpp through the `whisper-rs` crate) behind a Cargo feature, or as an external local executable the user configures. The model is loaded only while dictating and unloaded afterwards, so idle RAM stays within §9.
   - UX: a mic button / push-to-talk key inside the caption field and quick-note input; text streams into the native input, so Ctrl+Backspace and all editing still work. Optional wheel wedge for "dictate a note".
   - Audio is processed in memory and never written to disk or the library.
2. **Embeddings (deferred).** Semantic search, related items and tag suggestions in the radial. Revisit once the library has tagged data. Options: a bundled ~25 MB text model (fits the 50 MB cap), or an optional user-supplied CLIP-style image model in a models folder (never downloaded by the app). Vectors would live in a disposable `.index/` cache. M6/M7 must not make this harder.
3. **UI polish.** Anti-aliased wheel and overlay (per-pixel-alpha layered windows or Direct2D instead of colour-keyed GDI), open/close animations, popup transitions, refined typography/icons, light/dark themes, high-contrast and reduced-motion support, DPI/multi-monitor visual QA, app/tray icon design.

### UI polish plan (direction: Fluent glass, accent follows Windows)
Principles: dark default + light via system; Mica/translucent layers, soft borders, radii 8/12/16, 8 px grid, Segoe UI Variable; motion 120-180 ms ease-out, only after the save is done, respects reduced-motion; one token set (src/theme.css, mirrored as constants for native windows).
- **P1 Foundation (done):** theme.css tokens (accent = CSS AccentColor), app icon + tray states (normal / paused / hotkey-unavailable).
- **P2 Gallery + Settings (done; viewer/hover actions/filters untested by hand):** Mica window, slim top bar, pill search, left rail (tags, apps, date filters), hover actions, fade-in cards, arrow-key viewer, empty state; settings as grouped cards with toggles and keycap chips.
- **P3 Popup + Note (done):** slide-in, rounded card + shadow, countdown bar, metadata line, smooth caption expand.
- **P4 Wheel (done, 9 wedges: Settings added, constant-width parallel gaps) + Snip overlay (done: accent border, corner brackets, rounded size pill, glass-tinted dim):** Direct2D/DirectWrite per-pixel-alpha rendering (fallback: supersampled GDI); icon wedges, centre label, open/hover animation; accent border, corner handles, rounded size pill.
- **Themes (M6: Liquid Glass only):** System / Light / Dark are removed. The restyle follows reference images in `reference/liquid-glass/` chosen by the user. Previous behaviour, kept for reference: System / Light / Dark / Liquid Glass, chosen in Settings (applies live). Web windows read data-theme from src/theme.ts; gallery/settings backdrop = Mica, or Acrylic for glass (M6+: our own composition backdrop, see §7); the native wheel renders per-pixel-alpha with tiny-skia and, in glass, frosts a blurred copy of the frozen screen behind it. Liquid Glass gradient colours (3 stops) are configurable in Settings and drive the web windows, the wheel sheen, the snip pill and the Acrylic tint. One accent source (theme.rs): Windows accent, or brand blue if it is neutral grey.
- Guards: overlay/wheel still < 150 ms (measure each step); transparent webview windows fall back to solid cards if they flicker; Mica falls back to flat dark.

## 12. Open questions
1. Final name; is `Ctrl+Alt+S` free in the user's other tools?
2. Sync story: plain OneDrive/Dropbox folder (assumed) vs. none.
