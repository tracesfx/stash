import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { startDrag } from "@crabnebula/tauri-plugin-drag";

interface Item {
  id: string; kind: string; created: string; method: string; caption: string; tags: string[];
  source_title: string; process: string; url: string | null; ext: string; name: string; bytes: number;
  mime: string; w: number | null; h: number | null; original_path: string | null;
  text: string | null; path: string; icon: string;
}

const BATCH = 120;
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const grid = $("grid"), wrap = $("grid-wrap"), detail = $("detail-in"), q = $<HTMLInputElement>("q");
const empty = $("empty"), count = $("count"), toast = $("toast"), dropEl = $("drop"), rail = $("rail"), viewer = $("viewer");

// Static icon markup only (never user data).
const SVG = (d: string) => `<svg class="ico" viewBox="0 0 16 16">${d}</svg>`;
const ICON = {
  copy: SVG('<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M3 10.5V4.5A1.5 1.5 0 0 1 4.5 3h6"/>'),
  trash: SVG('<path d="M2.5 4.5h11M6 4.5V3h4v1.5M4 4.5l.6 8.5h6.8l.6-8.5"/>'),
  open: SVG('<path d="M7 3.5H4A1.5 1.5 0 0 0 2.5 5v7A1.5 1.5 0 0 0 4 13.5h7a1.5 1.5 0 0 0 1.5-1.5V9M9.5 2.5h4v4M13.5 2.5l-6 6"/>'),
};

let items: Item[] = [];
let shown: Item[] = [];
let rendered = 0;
let selected = new Set<string>();
let anchor = -1;
let dragging = false;
let toastTimer: number | undefined;
// filters
const f = { date: "all", type: "all", tag: "", app: "" };
let viewIdx = -1;

const thumb = (it: Item) => `http://stash.localhost/thumb/${it.id}`;
const full = (it: Item) => `http://stash.localhost/full/${it.id}`;
const isImg = (it: Item) => it.kind === "image";
const canCopy = (it: Item) => it.ext.toLowerCase() === "png" || it.kind === "text" || it.kind === "link";

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text; // textContent only: captions/titles are untrusted
  return e;
}

// ---------- data ----------
async function load(force = false) {
  const next = await invoke<Item[]>("gallery_list");
  // Re-rendering on every window focus would reset scroll/selection; only do it when the library changed.
  if (!force && items.length && next.length === items.length && next[0]?.id === items[0]?.id) return;
  items = next;
  selected = new Set([...selected].filter((id) => items.some((i) => i.id === id)));
  renderRail();
  filter();
}

function haystack(it: Item) {
  return [it.caption, it.tags.join(" "), it.source_title, it.process, it.url ?? "", it.name, it.text ?? "", it.kind, it.method, it.created.slice(0, 10)].join("\n").toLowerCase();
}

function inDate(it: Item): boolean {
  if (f.date === "all") return true;
  const age = Date.now() - new Date(it.created).getTime();
  const day = 86400000;
  return f.date === "today" ? age < day : f.date === "week" ? age < 7 * day : age < 30 * day;
}

function filter() {
  const terms = q.value.toLowerCase().split(/\s+/).filter(Boolean);
  shown = items.filter((it) => {
    if (f.type !== "all" && it.kind !== f.type) return false;
    if (f.tag && !it.tags.some((t) => t.toLowerCase() === f.tag.toLowerCase())) return false;
    if (f.app && it.process !== f.app) return false;
    if (!inDate(it)) return false;
    const h = haystack(it);
    return terms.every((t) => {
      if (t.startsWith("#")) return it.tags.some((x) => x.toLowerCase().includes(t.slice(1)));
      if (t.startsWith("type:")) return it.kind.startsWith(t.slice(5));
      return h.includes(t);
    });
  });
  render();
}

// ---------- rail ----------
function railItem(label: string, n: number | null, on: boolean, onclick: () => void) {
  const b = el("button", "ri" + (on ? " on" : ""));
  b.append(el("span", "", label));
  if (n !== null) b.append(el("span", "n", String(n)));
  b.onclick = onclick;
  return b;
}
function top<T>(m: Map<T, number>, n: number): [T, number][] {
  return [...m.entries()].sort((a, b) => b[1] - a[1]).slice(0, n);
}

function renderRail() {
  rail.replaceChildren();
  const section = (title: string, nodes: HTMLElement[]) => { if (nodes.length) rail.append(el("div", "rh", title), ...nodes); };
  const set = (k: keyof typeof f, v: string) => () => { f[k] = f[k] === v && k !== "date" && k !== "type" ? "" : v; if (k === "date" && v === f.date) f.date = "all"; renderRail(); filter(); };

  section("Library", [["all", "All items"], ["today", "Today"], ["week", "This week"], ["month", "This month"]].map(([v, l]) =>
    railItem(l, v === "all" ? items.length : null, f.date === v, () => { f.date = v; renderRail(); filter(); })));
  const types: [string, string][] = [["all", "Everything"], ["image", "Images"], ["text", "Text"], ["link", "Links"], ["file", "Files"]];
  section("Type", types.map(([v, l]) =>
    railItem(l, v === "all" ? null : items.filter((i) => i.kind === v).length, f.type === v, () => { f.type = v; renderRail(); filter(); })));

  const tags = new Map<string, number>(), apps = new Map<string, number>();
  for (const it of items) {
    it.tags.forEach((t) => tags.set(t, (tags.get(t) ?? 0) + 1));
    if (it.process) apps.set(it.process, (apps.get(it.process) ?? 0) + 1);
  }
  section("Tags", top(tags, 12).map(([t, n]) => railItem("#" + t, n, f.tag === t, set("tag", t))));
  section("Apps", top(apps, 8).map(([a, n]) => railItem(a.replace(/\.exe$/i, ""), n, f.app === a, set("app", a))));
}

// ---------- grid ----------
function actBtn(icon: string, title: string, fn: () => void, cls = ""): HTMLButtonElement {
  const b = el("button", "ab " + cls);
  b.innerHTML = icon; // static markup
  b.title = title;
  b.addEventListener("click", (e) => { e.stopPropagation(); fn(); });
  b.addEventListener("mousedown", (e) => e.stopPropagation());
  b.draggable = false;
  return b;
}

function card(it: Item): HTMLElement {
  const c = el("div", "card" + (selected.has(it.id) ? " sel" : ""));
  c.dataset.id = it.id;
  c.draggable = true;
  if (isImg(it)) {
    const img = el("img");
    img.loading = "lazy";
    img.draggable = false;
    if (it.w && it.h) img.style.aspectRatio = `${it.w} / ${it.h}`;
    img.onload = () => img.classList.add("ok");
    img.src = thumb(it);
    c.append(img);
  } else if (it.kind === "text" || it.kind === "link") {
    c.append(el("div", "t" + (it.kind === "link" ? " link" : ""), it.text ?? ""));
  } else {
    const fl = el("div", "f");
    fl.append(el("span", "badge", it.ext || "file"), el("span", "", it.original_path?.split(/[\\/]/).pop() ?? it.name));
    c.append(fl);
  }
  if (it.caption) c.append(el("div", "cap", it.caption));
  const acts = el("div", "acts");
  if (canCopy(it)) acts.append(actBtn(ICON.copy, "Copy", () => copyItem(it)));
  if (it.url) acts.append(actBtn(ICON.open, "Open source page", () => invoke("gallery_open_url", { url: it.url })));
  acts.append(actBtn(ICON.trash, "Move to trash", () => trash([it.id]), "d"));
  c.append(acts);
  return c;
}

function render() {
  grid.replaceChildren();
  rendered = 0;
  wrap.scrollTop = 0;
  more();
  count.textContent = shown.length === items.length ? `${items.length} items` : `${shown.length} of ${items.length}`;
  empty.classList.toggle("on", shown.length === 0);
  if (!shown.length) {
    empty.replaceChildren();
    empty.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M5 9V5h4M15 5h4v4M19 15v4h-4M9 19H5v-4"/><circle cx="12" cy="12" r="1.4" fill="var(--accent)" stroke="none"/></svg>';
    if (items.length) {
      empty.append(el("h2", "", "Nothing matches"), el("div", "", "Try clearing a filter or the search."));
    } else {
      const hint = el("div");
      hint.append("Tap ", Object.assign(el("kbd"), { textContent: "Ctrl" }), " ", Object.assign(el("kbd"), { textContent: "Alt" }), " ", Object.assign(el("kbd"), { textContent: "S" }), " to snip, hold it for the wheel, or drop files here.");
      empty.append(el("h2", "", "Your library is empty"), hint);
    }
  }
}

function more() {
  const frag = document.createDocumentFragment();
  for (const it of shown.slice(rendered, rendered + BATCH)) frag.append(card(it));
  rendered = Math.min(shown.length, rendered + BATCH);
  grid.append(frag);
}
new IntersectionObserver((e) => { if (e[0].isIntersecting && rendered < shown.length) more(); }, { root: wrap, rootMargin: "800px" }).observe($("sentinel"));

function refreshSel() {
  grid.querySelectorAll<HTMLElement>(".card").forEach((c) => c.classList.toggle("sel", selected.has(c.dataset.id!)));
  showDetail();
}

// ---------- selection / drag / viewer ----------
grid.addEventListener("click", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (!c) return;
  const idx = shown.findIndex((i) => i.id === c.dataset.id);
  if (e.shiftKey && anchor >= 0) {
    const [a, b] = [Math.min(anchor, idx), Math.max(anchor, idx)];
    selected = new Set(shown.slice(a, b + 1).map((i) => i.id));
  } else if (e.ctrlKey || e.metaKey) {
    selected.has(c.dataset.id!) ? selected.delete(c.dataset.id!) : selected.add(c.dataset.id!);
    anchor = idx;
  } else {
    selected = new Set([c.dataset.id!]);
    anchor = idx;
  }
  refreshSel();
});
grid.addEventListener("dblclick", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (c) openViewer(shown.findIndex((i) => i.id === c.dataset.id));
});

grid.addEventListener("dragstart", async (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (!c) return;
  e.preventDefault(); // replace the HTML5 drag with a native file drag
  const id = c.dataset.id!;
  const ids = selected.has(id) ? [...selected] : [id];
  const picked = ids.map((i) => items.find((x) => x.id === i)!).filter(Boolean);
  dragging = true;
  try {
    await startDrag({ item: picked.map((p) => p.path), icon: picked[0].icon });
  } catch (err) {
    console.error(err);
  } finally {
    setTimeout(() => (dragging = false), 400);
  }
});

function openViewer(i: number) {
  if (i < 0 || i >= shown.length) return;
  viewIdx = i;
  const it = shown[i];
  viewer.replaceChildren();
  if (isImg(it)) {
    const img = el("img");
    img.src = it.ext.toLowerCase() === "png" || it.ext.toLowerCase() === "jpg" ? full(it) : thumb(it);
    viewer.append(img);
  } else {
    viewer.append(el("div", "vt", it.text ?? it.name));
  }
  if (it.caption) viewer.append(el("div", "vc", it.caption));
  for (const [dir, label, side] of [[-1, "‹", "left"], [1, "›", "right"]] as const) {
    const b = el("button", "vn", label);
    b.style[side] = "22px";
    b.onclick = (e) => { e.stopPropagation(); openViewer(viewIdx + dir); };
    viewer.append(b);
  }
  viewer.classList.add("on");
}
viewer.addEventListener("click", () => { viewer.classList.remove("on"); viewIdx = -1; });

// ---------- detail panel ----------
function fmtBytes(n: number) { return n > 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(n / 1e3)) + " KB"; }

function showDetail() {
  detail.replaceChildren();
  const sel = [...selected].map((id) => items.find((i) => i.id === id)!).filter(Boolean);
  document.body.classList.toggle("detail", sel.length > 0);
  if (!sel.length) return;
  if (sel.length > 1) {
    detail.append(el("h3", "", `${sel.length} selected`));
    const del = el("button", "btn danger", "Move to trash");
    del.onclick = () => trash(sel.map((s) => s.id));
    detail.append(del);
    return;
  }
  const it = sel[0];
  if (isImg(it)) {
    const pv = el("img", "pv");
    pv.src = it.ext.toLowerCase() === "png" ? full(it) : thumb(it);
    pv.onclick = () => openViewer(shown.findIndex((x) => x.id === it.id));
    detail.append(pv);
  } else if (it.text) {
    detail.append(el("div", "tx", it.text));
  }

  const cap = el("input", "field"); cap.type = "text"; cap.value = it.caption; cap.placeholder = "Add a caption";
  const tags = el("input", "field"); tags.type = "text"; tags.value = it.tags.map((t) => "#" + t).join(" "); tags.placeholder = "#tag #another";
  const orig = tags.value;
  const save = async () => {
    const t = tags.value.split(/[\s,]+/).filter(Boolean);
    if (cap.value.trim() === it.caption && t.map((x) => x.replace(/^#/, "")).join("|") === it.tags.join("|")) return;
    await invoke("gallery_set_meta", { id: it.id, caption: cap.value, tags: t });
    it.caption = cap.value.trim();
    it.tags = [...new Set(t.map((x) => x.replace(/^#/, "")))];
    const cardEl = grid.querySelector<HTMLElement>(`.card[data-id="${it.id}"]`);
    if (cardEl) { const fresh = card(it); fresh.classList.add("sel"); cardEl.replaceWith(fresh); }
    renderRail();
  };
  for (const inp of [cap, tags]) {
    inp.addEventListener("keydown", (e) => {
      if (e.key === "Enter") { e.preventDefault(); inp.blur(); }
      else if (e.key === "Escape") { inp.value = inp === cap ? it.caption : orig; inp.blur(); e.stopPropagation(); }
    });
    inp.addEventListener("blur", save);
  }
  detail.append(el("label", "l", "Caption"), cap, el("label", "l", "Tags"), tags);

  const row = el("div", "row");
  const mk = (label: string, fn: () => void, cls = "") => { const b = el("button", "btn " + cls, label); b.onclick = fn; row.append(b); };
  if (canCopy(it)) mk("Copy", () => copyItem(it));
  mk("Show in folder", () => invoke("gallery_reveal", { id: it.id }));
  if (it.url) mk("Open source", () => invoke("gallery_open_url", { url: it.url }));
  mk("Trash", () => trash([it.id]), "danger");
  detail.append(row);

  const meta = el("div", "meta");
  const add = (k: string, v: string | HTMLElement) => { const b = el("b"); typeof v === "string" ? (b.textContent = v) : b.append(v); meta.append(el("span", "", k), b); };
  add("Saved", new Date(it.created).toLocaleString());
  add("How", it.method);
  if (it.w && it.h) add("Size", `${it.w} × ${it.h}`);
  add("File", `${it.ext.toUpperCase()} · ${fmtBytes(it.bytes)}`);
  if (it.process) add("App", it.process);
  if (it.source_title) add("Window", it.source_title);
  if (it.url) { const a = el("a", "", it.url); a.onclick = () => invoke("gallery_open_url", { url: it.url }); add("URL", a); }
  if (it.original_path) add("From", it.original_path);
  detail.append(meta);
}

// ---------- actions ----------
function flash(msg: string, undo?: () => void) {
  toast.replaceChildren(el("span", "", msg));
  if (undo) { const a = el("a", "", "Undo"); a.onclick = () => { undo(); toast.classList.remove("on"); }; toast.append(a); }
  toast.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("on"), undo ? 6000 : 2200);
}

const copyItem = (it: Item) => invoke("gallery_copy", { id: it.id }).then(() => flash("Copied")).catch((e) => flash(String(e)));

async function trash(ids: string[]) {
  const moved = await invoke<string[]>("gallery_delete", { ids });
  selected.clear();
  items = items.filter((i) => !moved.includes(i.id));
  renderRail();
  filter();
  showDetail();
  flash(`Moved ${moved.length} to trash`, async () => { await invoke("gallery_restore", { ids: moved }); await load(true); });
}

// ---------- events ----------
q.addEventListener("input", filter);

window.addEventListener("keydown", (e) => {
  const inField = (e.target as HTMLElement).tagName === "INPUT";
  if (viewer.classList.contains("on")) {
    if (e.key === "Escape") { viewer.classList.remove("on"); }
    else if (e.key === "ArrowRight") openViewer(viewIdx + 1);
    else if (e.key === "ArrowLeft") openViewer(viewIdx - 1);
    return;
  }
  if (e.ctrlKey && e.key.toLowerCase() === "f") { e.preventDefault(); q.focus(); q.select(); }
  else if (e.key === "/" && !inField) { e.preventDefault(); q.focus(); }
  else if (e.key === "Escape") { if (inField) (e.target as HTMLElement).blur(); else { selected.clear(); refreshSel(); } }
  else if (!inField && e.key === "Delete" && selected.size) trash([...selected]);
  else if (!inField && (e.key === " " || e.key === "Enter") && selected.size === 1) { e.preventDefault(); openViewer(shown.findIndex((i) => i.id === [...selected][0])); }
  else if (!inField && e.ctrlKey && e.key.toLowerCase() === "a") { e.preventDefault(); selected = new Set(shown.map((i) => i.id)); refreshSel(); }
});

// Files dropped from Explorer are imported; ignore our own drag-out coming back in.
getCurrentWebview().onDragDropEvent(async (ev) => {
  const p = ev.payload;
  if (p.type === "enter" || p.type === "over") dropEl.classList.toggle("on", !dragging);
  else if (p.type === "leave") dropEl.classList.remove("on");
  else if (p.type === "drop") {
    dropEl.classList.remove("on");
    if (dragging || !p.paths.length) return;
    const n = await invoke<number>("gallery_import", { paths: p.paths });
    flash(`Added ${n} file${n === 1 ? "" : "s"}`);
    await load(true);
  }
});

window.addEventListener("focus", () => load()); // pick up captures made while the window was in the background
load(true);
q.focus();
