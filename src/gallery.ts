// Gallery (SPEC §6, M7): Notes / Visuals / All / Favourites / Canvases / Trash tabs, one continuous masonry with day labels
// (short days share rows), tag collections, chip tagging, FLIP-animated layout changes, saved canvases and a trash
// that empties itself. User text is only ever set through textContent.
import "./theme";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";

interface Item {
  id: string; kind: string; created: string; method: string; caption: string; tags: string[];
  source_title: string; process: string; url: string | null; ext: string; name: string; bytes: number;
  mime: string; w: number | null; h: number | null; original_path: string | null;
  text: string | null; path: string; icon: string;
  palette: string[]; ocr: string | null; trashed?: string | null; favorite?: boolean;
}
type Tab = "notes" | "visuals" | "all" | "favs" | "canvases" | "trash";
interface CanvasInfo { id: string; name: string; created: string; modified: string; ids: string[] }
interface CanvasDoc extends CanvasInfo { v: 2; pos: Record<string, [number, number]>; labels: { t: string; x: number; y: number }[]; pan: [number, number] | null; zoom: number }

const PAGE = 160; // cards added per step of infinite scroll
const LABEL_H = 30;
const EASE = "cubic-bezier(.2, .8, .2, 1)", SPRING = "cubic-bezier(.34, 1.4, .5, 1)";
/** Re-arranging (FLIP): a gentle ease in and a long, soft settle. */
const MOVE_MS = 420, MOVE_EASE = "cubic-bezier(.45, 0, .2, 1)";
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const grid = $("grid"), wrap = $("grid-wrap"), detail = $("detail-in"), q = $<HTMLInputElement>("q");
const empty = $("empty"), count = $("count"), toast = $("toast"), dropEl = $("drop"), rail = $("rail"), viewer = $("viewer");
const chipsEl = $("chips"), scrub = $("scrub"), canvasEl = $("canvas"), world = $("cv-world"), pop = $("tagpop");
const tbar = $("tbar"), daypill = $("daypill"), cvhome = $("cvhome"), cvHead = $("cv-head");

// Static icon markup only (never user data).
const SVG = (d: string) => `<svg class="ico" viewBox="0 0 16 16">${d}</svg>`;
const ICON = {
  copy: SVG('<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M3 10.5V4.5A1.5 1.5 0 0 1 4.5 3h6"/>'),
  trash: SVG('<path d="M2.5 4.5h11M6 4.5V3h4v1.5M4 4.5l.6 8.5h6.8l.6-8.5"/>'),
  open: SVG('<path d="M7 3.5H4A1.5 1.5 0 0 0 2.5 5v7A1.5 1.5 0 0 0 4 13.5h7a1.5 1.5 0 0 0 1.5-1.5V9M9.5 2.5h4v4M13.5 2.5l-6 6"/>'),
  link: SVG('<path d="M6.5 9.5l3-3M7 4.5l1-1a2.5 2.5 0 0 1 3.5 3.5l-1 1M9 11.5l-1 1A2.5 2.5 0 0 1 4.5 9l1-1"/>'),
  restore: SVG('<path d="M3 8a5 5 0 1 0 1.5-3.5M3 2.5v2.5h2.5"/>'),
  tag: SVG('<path d="M2.5 8.2V3a.5.5 0 0 1 .5-.5h5.2l5.3 5.3-5.7 5.7z"/><circle cx="5.5" cy="5.5" r="1"/>'),
  selAll: SVG('<rect x="2.5" y="2.5" width="11" height="11" rx="2"/><path d="M5.5 8l1.8 1.8L10.8 6"/>'),
  exp: SVG('<path d="M8 2v8M5 7l3 3 3-3M3 13.5h10"/>'),
  close: SVG('<path d="M4 4l8 8M12 4l-8 8"/>'),
  plus: SVG('<path d="M8 3v10M3 8h10"/>'),
  star: SVG('<path d="M8 1.9l1.83 3.86 4.22.53-3.1 2.9.8 4.18L8 11.3l-3.75 2.07.8-4.18-3.1-2.9 4.22-.53z"/>'),
  canvas: '<svg viewBox="0 0 16 16"><rect x="2" y="2.5" width="5" height="4.5" rx="1"/><rect x="9" y="2.5" width="5" height="7" rx="1"/><rect x="2" y="9" width="5" height="4.5" rx="1"/><path d="M9 12.5h5"/></svg>',
};

let items: Item[] = [];
let trashItems: Item[] = [];
let canvases: CanvasInfo[] = [];
let shown: Item[] = [];
let limit = PAGE;
let selected = new Set<string>();
let anchor = -1;
let dragging = false;
let toastTimer: number | undefined;
const f = { date: "all", day: "", tag: "", app: "", color: "" };
let viewIdx = -1;
let opening = true;
let trashDays = 30;

const store = { get: (k: string) => { try { return localStorage.getItem(k); } catch { return null; } }, set: (k: string, v: string) => { try { localStorage.setItem(k, v); } catch { /* storage unavailable */ } } };
/** Gallery display (the View menu in the header; remembered on this PC). `size` = minimum card width. */
interface ViewPrefs { size: number; gap: number; captions: boolean; days: boolean; sources: boolean; square: boolean }
const VIEW_DEFAULT: ViewPrefs = { size: 220, gap: 14, captions: true, days: true, sources: true, square: false };
const view: ViewPrefs = (() => { try { return { ...VIEW_DEFAULT, ...JSON.parse(store.get("stash.gallery.view") ?? "{}") }; } catch { return { ...VIEW_DEFAULT }; } })();
const TABS: [Tab, string][] = [["notes", "Notes"], ["visuals", "Visuals"], ["all", "All"], ["favs", "Favourites"], ["canvases", "Canvases"], ["trash", "Trash"]];
let tab: Tab = (TABS.some(([t]) => t === store.get("stash.gallery.tab")) ? store.get("stash.gallery.tab") : "all") as Tab;
const itemTab = () => tab === "notes" || tab === "visuals" || tab === "all" || tab === "favs";

const enc = encodeURIComponent;
const thumb = (it: Item) => `http://stash.localhost/thumb/${enc(it.id)}`;
const full = (it: Item) => `http://stash.localhost/full/${enc(it.id)}`;
const isImg = (it: Item) => it.kind === "image";
const isNote = (it: Item) => it.kind === "text" || it.kind === "link";
const VIDEO = new Set(["mp4", "m4v", "webm", "mov"]);
const AUDIO = new Set(["mp3", "wav", "ogg", "oga", "m4a", "flac"]);
const ext = (it: Item) => it.ext.toLowerCase();
const isVideo = (it: Item) => it.kind !== "image" && VIDEO.has(ext(it));
const isAudio = (it: Item) => it.kind !== "image" && AUDIO.has(ext(it));
const isPdf = (it: Item) => ext(it) === "pdf";
const isTextFile = (it: Item) => it.kind === "file" && it.text != null;
const canCopy = (it: Item) => ext(it) === "png" || it.kind === "text" || it.kind === "link";
const reduced = () => document.documentElement.dataset.motion === "off" || matchMedia("(prefers-reduced-motion: reduce)").matches;
const byId = (id: string) => items.find((i) => i.id === id) ?? (tab === "trash" ? trashItems.find((i) => i.id === id) : undefined);

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text; // textContent only: captions/titles are untrusted
  return e;
}

// ---------- colours ----------
const BUCKETS: [string, string][] = [
  ["red", "#e5484d"], ["orange", "#f08a24"], ["yellow", "#f5d90a"], ["green", "#3fb950"], ["teal", "#1fc2b5"], ["blue", "#3b82f6"],
  ["purple", "#8b5cf6"], ["pink", "#ec4899"], ["brown", "#8b5a3c"], ["gray", "#8a8f98"], ["black", "#16161a"], ["white", "#f4f4f6"],
];
function bucketOf(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  const mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, d = mx - mn;
  const sat = d === 0 ? 0 : d / (1 - Math.abs(2 * l - 1));
  if (l < 0.14) return "black";
  if (l > 0.9) return "white";
  if (sat < 0.14) return "gray";
  let h = mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4;
  h = (h * 60 + 360) % 360;
  if (h >= 12 && h < 48 && l < 0.4) return "brown";
  if (h < 12 || h >= 345) return "red";
  if (h < 40) return "orange";
  if (h < 68) return "yellow";
  if (h < 165) return "green";
  if (h < 200) return "teal";
  if (h < 255) return "blue";
  if (h < 295) return "purple";
  return "pink";
}
const hasColor = (it: Item, name: string) => it.palette.slice(0, 3).some((c) => bucketOf(c) === name);

// ---------- days ----------
const pad = (n: number) => String(n).padStart(2, "0");
const keyOf = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const dayCache = new Map<string, string>();
function dayKey(it: Item): string {
  const when = tab === "trash" ? it.trashed ?? it.created : it.created;
  const ck = (tab === "trash" ? "t:" : "") + it.id;
  let k = dayCache.get(ck);
  if (!k) { k = keyOf(new Date(when)); dayCache.set(ck, k); }
  return k;
}
function dayLabel(key: string, long = false): string {
  const [y, m, d] = key.split("-").map(Number);
  const date = new Date(y, m - 1, d);
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const diff = Math.round((today.getTime() - date.getTime()) / 86400000);
  let s: string;
  if (diff === 0) s = "Today";
  else if (diff === 1) s = "Yesterday";
  else {
    const opts: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short" };
    if (date.getFullYear() !== today.getFullYear() || long) opts.year = "numeric";
    s = date.toLocaleDateString(undefined, opts);
  }
  return tab === "trash" && !long ? `Deleted ${diff <= 1 ? s.toLowerCase() : s}` : s;
}
const timeOf = (it: Item) => new Date(it.created).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
function ago(iso: string): string {
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 50) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  const d = Math.round(s / 86400);
  return d === 1 ? "yesterday" : `${d} days ago`;
}

// ---------- data ----------
const sig = (l: Item[]) => l.map((i) => i.id + cardVer(i)).join();
async function load(animate = false) {
  const next = await invoke<Item[]>("gallery_list");
  if (items.length && sig(next) === sig(items)) return; // nothing changed: keep scroll, selection and cards
  items = next;
  pruneCards();
  if (tab !== "trash") selected = new Set([...selected].filter((id) => items.some((i) => i.id === id)));
  renderRail();
  if (tab === "canvases") { if (!cv.doc) renderCanvases(); } // an open canvas is rebuilt by the caller
  else if (tab !== "trash") filter(animate ? "flip" : "none");
  updateSelBar();
  backfill();
}
async function loadTrash(animate = false) {
  const next = await invoke<Item[]>("gallery_trash_list").catch(() => [] as Item[]);
  const changed = sig(next) !== sig(trashItems);
  trashItems = next;
  if (tab === "trash" && (changed || !animate)) { pruneCards(); renderRail(); filter(animate ? "flip" : "none"); }
}
async function loadCanvases() {
  canvases = await invoke<CanvasInfo[]>("canvas_list").catch(() => []);
}
function pruneCards() {
  const ids = new Set([...items, ...trashItems].map((i) => i.id));
  for (const k of [...cardCache.keys()]) if (!ids.has(k.slice(k.indexOf(":") + 1))) cardCache.delete(k);
}

function haystack(it: Item) {
  return [it.caption, it.tags.join(" "), it.source_title, it.process, it.url ?? "", it.name, it.text ?? "", it.ocr ?? "", it.kind, it.method, it.created.slice(0, 10)].join("\n").toLowerCase();
}

function inDate(it: Item): boolean {
  if (f.day) return dayKey(it) === f.day;
  if (f.date === "all") return true;
  const age = Date.now() - new Date(it.created).getTime();
  const day = 86400000;
  return f.date === "today" ? keyOf(new Date(it.created)) === keyOf(new Date()) : f.date === "week" ? age < 7 * day : age < 30 * day;
}
const inTab = (it: Item) => tab === "all" || tab === "trash" || (tab === "favs" ? !!it.favorite : (tab === "notes") === isNote(it));
const pool = () => (tab === "trash" ? trashItems : items.filter(inTab));
const hasTag = (it: Item, t: string) => it.tags.some((x) => x.toLowerCase() === t.toLowerCase());

/** Recomputes the shown list and re-lays out. `anim`: "none" (instant), "flip" (keep cards, animate the
 *  difference), "switch" (as flip from the top, new cards wait briefly for their images so they enter complete). */
function filter(anim: Anim = "none") {
  if (!itemTab() && tab !== "trash") return;
  const terms = q.value.toLowerCase().split(/\s+/).filter(Boolean);
  shown = pool().filter((it) => {
    if (f.tag && !hasTag(it, f.tag)) return false;
    if (f.app && it.process !== f.app) return false;
    if (f.color && !hasColor(it, f.color)) return false;
    if (!inDate(it)) return false;
    const h = haystack(it);
    return terms.every((t) => {
      if (t.startsWith("#")) return it.tags.some((x) => x.toLowerCase().includes(t.slice(1)));
      if (t.startsWith("type:")) return it.kind.startsWith(t.slice(5));
      if (t.startsWith("color:")) return it.palette.slice(0, 3).some((c) => bucketOf(c).startsWith(t.slice(6)));
      if (t.startsWith("app:")) return it.process.toLowerCase().replace(/\.exe$/, "").startsWith(t.slice(4));
      return h.includes(t);
    });
  });
  if (anim === "switch") limit = PAGE;
  limit = Math.max(PAGE, Math.min(limit, shown.length));
  renderChips();
  renderTrashBar();
  const total = pool().length;
  count.textContent = tab === "trash" ? `${total} in trash` : shown.length === total ? `${total} items` : `${shown.length} of ${total}`;
  renderEmpty();
  if (anim === "switch") { wrap.scrollTop = 0; smooth.stop(); }
  layout(opening ? "none" : anim);
  renderScrub();
}

function renderEmpty() {
  empty.classList.toggle("on", shown.length === 0);
  if (shown.length) return;
  empty.replaceChildren();
  empty.innerHTML = '<svg viewBox="0 0 24 24" fill="none" stroke="var(--accent)" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M5 9V5h4M15 5h4v4M19 15v4h-4M9 19H5v-4"/><circle cx="12" cy="12" r="1.4" fill="var(--accent)" stroke="none"/></svg>';
  if (tab === "trash") {
    empty.append(el("h2", "", "The trash is empty"), el("div", "", "Deleted items wait here so you can restore them."));
  } else if (tab === "favs" && !items.some((i) => i.favorite)) {
    empty.append(el("h2", "", "No favourites yet"), el("div", "", "Star anything with the ☆ on its card, F, or the star in the middle of the tag menu after a capture."));
  } else if (items.length) {
    empty.append(el("h2", "", "Nothing here"), el("div", "", tab === "notes" ? "No notes match. Try All, or clear a filter." : "Try another tab, or clear a filter or the search."));
  } else {
    const hint = el("div");
    hint.append("Tap ", Object.assign(el("kbd"), { textContent: "Ctrl" }), " ", Object.assign(el("kbd"), { textContent: "Alt" }), " ", Object.assign(el("kbd"), { textContent: "S" }), " to snip, hold it for the wheel, or drop / paste (Ctrl+V) anything here.");
    empty.append(el("h2", "", "Your library is empty"), hint);
  }
}

// ---------- header: tabs, filter chips, trash bar ----------
function seg(host: HTMLElement, opts: [string, string][], cur: () => string, pick: (v: string) => void) {
  const ind = el("span", "ind");
  host.append(ind);
  const btns: HTMLButtonElement[] = [];
  const move = () => {
    const b = btns.find((x) => x.dataset.v === cur());
    if (!b) return;
    ind.style.width = `${b.offsetWidth}px`;
    ind.style.transform = `translateX(${b.offsetLeft}px)`;
    btns.forEach((x) => x.classList.toggle("on", x === b));
  };
  for (const [v, label] of opts) {
    const b = el("button", "", label);
    b.dataset.v = v;
    b.setAttribute("role", "tab");
    b.onclick = () => { if (cur() !== v) { pick(v); move(); } else if (v === "canvases" && cv.doc) closeCanvasDoc(); };
    btns.push(b);
    host.append(b);
  }
  new ResizeObserver(move).observe(host);
  return move;
}
const moveTabInd = seg($("tabs"), TABS, () => tab, (v) => setTab(v as Tab));

function setTab(t: Tab, anim: Anim = "switch") {
  const was = tab;
  tab = t;
  store.set("stash.gallery.tab", t);
  if ((was === "trash") !== (t === "trash")) selected.clear();
  updateSelBar();
  showDetail();
  const canv = t === "canvases";
  wrap.style.display = canv ? "none" : "";
  chipsEl.style.display = canv ? "none" : "";
  if (canv) { scrub.classList.add("off"); daypill.classList.remove("on"); tbar.classList.remove("on"); count.textContent = `${canvases.length} canvas${canvases.length === 1 ? "" : "es"}`; }
  renderRail();
  if (canv) {
    const last = store.get("stash.gallery.canvas");
    if (cv.doc) showCanvasEditor(true);
    else if (last && canvases.some((c) => c.id === last)) openCanvasDoc(last);
    else renderCanvases();
    return;
  }
  cvhome.classList.remove("on");
  canvasEl.classList.remove("on");
  if (t === "trash") { loadTrash(); filter(anim); } else filter(anim);
  moveTabInd();
}

function renderChips() {
  chipsEl.replaceChildren();
  const chip = (label: string, clear: () => void) => {
    const c = el("button", "chip", label);
    const x = el("b", "", "×");
    c.append(x);
    c.title = "Remove filter";
    c.onclick = () => { clear(); renderRail(); filter("flip"); };
    chipsEl.append(c);
  };
  if (f.day) chip(dayLabel(f.day, true), () => (f.day = ""));
  else if (f.date !== "all") chip({ today: "Today", week: "This week", month: "This month" }[f.date] ?? f.date, () => (f.date = "all"));
  if (f.tag) { chip("#" + f.tag, () => (f.tag = "")); chipsEl.lastElementChild!.prepend(tagStar(f.tag)); }
  if (f.app) chip(f.app.replace(/\.exe$/i, ""), () => (f.app = ""));
  if (f.color) chip(f.color, () => (f.color = ""));
  if (chipsEl.childElementCount > 1) {
    const all = el("button", "chip", "Clear all");
    all.onclick = () => { Object.assign(f, { date: "all", day: "", tag: "", app: "", color: "" }); renderRail(); filter("flip"); };
    chipsEl.append(all);
  }
  chipsEl.classList.toggle("on", chipsEl.childElementCount > 0);
}

let emptyArmed = 0;
function renderTrashBar() {
  tbar.classList.toggle("on", tab === "trash");
  if (tab !== "trash") return;
  tbar.replaceChildren(el("span", "", trashDays > 0 ? `Items are deleted for good ${trashDays} day${trashDays === 1 ? "" : "s"} after you trash them (Settings › General).` : "Items stay here until you empty the trash."), el("span", "grow"));
  if (trashItems.length) {
    const b = el("button", "btn danger", Date.now() < emptyArmed ? "Click again to empty" : "Empty trash");
    b.onclick = async () => {
      if (Date.now() >= emptyArmed) { emptyArmed = Date.now() + 3000; renderTrashBar(); setTimeout(renderTrashBar, 3100); return; }
      emptyArmed = 0;
      const burning = burnCards(trashItems.map((i) => i.id));
      const n = await invoke<number>("gallery_purge", { ids: [] }).catch((e) => { flash(String(e)); return -1; });
      await burning;
      await burnsSettled();
      if (n < 0) { grid.querySelectorAll<HTMLElement>(".card.burning").forEach(unburn); return; }
      flash(`Deleted ${n} item${n === 1 ? "" : "s"} for good`);
      await loadTrash(true);
    };
    tbar.append(b);
  }
}

// ---------- rail: collections, dates, colours, apps (or the list of canvases) ----------
function railItem(label: string, n: number | null, on: boolean, onclick: () => void) {
  const b = el("button", "ri" + (on ? " on" : ""));
  b.append(el("span", "", label));
  if (n !== null) b.append(el("span", "n", String(n)));
  b.onclick = onclick;
  return b;
}
let showAllTags = false;
let favTags: string[] = [];
const isFavTag = (t: string) => favTags.some((x) => x.toLowerCase() === t.toLowerCase());
async function toggleFavTag(t: string) {
  const on = !isFavTag(t);
  favTags = on ? [...favTags, t] : favTags.filter((x) => x.toLowerCase() !== t.toLowerCase());
  renderRail();
  try { await invoke("gallery_set_fav_tags", { tags: favTags }); } catch (e) { flash(String(e)); return; }
  flash(on ? `#${t} pinned to Favourite tags` : `#${t} unpinned`);
}
/** Star inside a rail row or tile (a span: rows are buttons already). */
function tagStar(t: string): HTMLElement {
  const st = el("span", "tstar" + (isFavTag(t) ? " on" : ""));
  st.innerHTML = ICON.star; // static markup
  st.title = isFavTag(t) ? `Unpin #${t} from Favourite tags` : `Pin #${t} to Favourite tags`;
  st.addEventListener("click", (e) => { e.stopPropagation(); toggleFavTag(t); });
  return st;
}
function renderRail() {
  rail.replaceChildren();
  const section = (title: string, nodes: HTMLElement[]) => { if (nodes.length) rail.append(el("div", "rh", title), ...nodes); };
  if (tab === "canvases") {
    section("Canvases", [
      railItem("All canvases", canvases.length, !cv.doc, () => closeCanvasDoc()),
      ...canvases.map((c) => railItem(c.name || "Untitled canvas", c.ids.length, cv.doc?.id === c.id, () => openCanvasDoc(c.id))),
    ]);
    return;
  }
  const p = pool();

  // favourite tags: always first
  if (tab !== "trash" && favTags.length) {
    const counts = new Map<string, number>();
    for (const it of items) for (const t of it.tags) counts.set(t.toLowerCase(), (counts.get(t.toLowerCase()) ?? 0) + 1);
    section("Favourite tags", favTags.map((t) => {
      const b = railItem("#" + t, counts.get(t.toLowerCase()) ?? 0, f.tag.toLowerCase() === t.toLowerCase(), () => { f.tag = f.tag.toLowerCase() === t.toLowerCase() ? "" : t; renderRail(); filter("switch"); });
      b.classList.add("ft");
      b.prepend(tagStar(t));
      return b;
    }));
  }

  // collections: tags with a cover (newest image with that tag, else a text snippet)
  if (tab !== "trash") {
    const tagN = new Map<string, number>(), cover = new Map<string, Item>(), names = new Map<string, string>();
    for (const it of p) for (const t of it.tags) {
      const k = t.toLowerCase();
      tagN.set(k, (tagN.get(k) ?? 0) + 1);
      if (!names.has(k)) names.set(k, t);
      const c = cover.get(k);
      if (!c || (!isImg(c) && isImg(it))) cover.set(k, it);
    }
    const ranked = [...tagN.entries()].sort((a, b) => b[1] - a[1]);
    if (ranked.length) {
      const tiles = el("div", "cols");
      for (const [k, n] of ranked.slice(0, 6)) {
        const it = cover.get(k)!;
        const name = names.get(k)!;
        const b = el("button", "col" + (isImg(it) ? "" : " txt") + (f.tag.toLowerCase() === k ? " on" : ""));
        if (isImg(it)) b.style.backgroundImage = `url("${thumb(it)}")`;
        else b.append(el("span", "sn", (it.text ?? it.caption ?? "").slice(0, 120)));
        b.append(el("b", "", "#" + name), el("i", "", `${n} item${n === 1 ? "" : "s"}`));
        b.title = `#${name}`;
        b.onclick = () => { f.tag = f.tag.toLowerCase() === k ? "" : name; renderRail(); filter("switch"); };
        b.append(tagStar(name));
        tiles.append(b);
      }
      const rest = ranked.slice(6);
      const nodes: HTMLElement[] = [tiles];
      if (rest.length) {
        if (showAllTags) for (const [k, n] of rest) {
          const name = names.get(k)!;
          const r = railItem("#" + name, n, f.tag.toLowerCase() === k, () => { f.tag = f.tag.toLowerCase() === k ? "" : name; renderRail(); filter("switch"); });
          r.append(tagStar(name));
          nodes.push(r);
        }
        const more = el("button", "ri more", showAllTags ? "Fewer tags" : `${rest.length} more tag${rest.length === 1 ? "" : "s"}`);
        more.onclick = () => { showAllTags = !showAllTags; renderRail(); };
        nodes.push(more);
      }
      section("Collections", nodes);
    }
  }

  section("Library", [["all", "All time"], ["today", "Today"], ["week", "This week"], ["month", "This month"]].map(([v, l]) =>
    railItem(l, v === "all" ? p.length : null, !f.day && f.date === v, () => { f.date = v; f.day = ""; renderRail(); filter("switch"); })));

  if (opening) Array.from(rail.children).forEach((c, i) => (c as HTMLElement).style.setProperty("--i", String(Math.min(i, 24))));
}

// ---------- cards ----------
function actBtn(icon: string, title: string, fn: () => void, cls = ""): HTMLButtonElement {
  const b = el("button", "ab " + cls);
  b.innerHTML = icon; // static markup
  b.title = title;
  b.addEventListener("click", (e) => { e.stopPropagation(); fn(); });
  b.addEventListener("mousedown", (e) => e.stopPropagation());
  b.draggable = false;
  return b;
}
/** Star in the corner of a card: shows on hover, stays (gold) on favourites; click toggles. */
function favBtn(it: Item) {
  const b = el("button", "fb");
  b.innerHTML = ICON.star; // static markup
  b.title = it.favorite ? "Remove from Favourites (F)" : "Add to Favourites (F)";
  b.addEventListener("mousedown", (e) => e.stopPropagation());
  b.addEventListener("click", (e) => { e.stopPropagation(); setFav([it.id], !it.favorite); });
  return b;
}
function sourceLabel(it: Item): string {
  if (it.source_title) return it.source_title;
  if (it.url) { try { return new URL(it.url).hostname; } catch { return it.url; } }
  return it.process.replace(/\.exe$/i, "");
}
function daysLeft(it: Item): string {
  if (!it.trashed) return "";
  const left = trashDays - Math.floor((Date.now() - new Date(it.trashed).getTime()) / 86400000);
  return trashDays > 0 ? ` · ${Math.max(0, left)} day${left === 1 ? "" : "s"} left` : "";
}

// Card elements are cached per layout kind and re-used, so their images stay decoded across view changes.
const cardCache = new Map<string, { el: HTMLElement; v: string }>();
const cardVer = (it: Item) => `${it.caption}|${it.tags.join(",")}|${it.palette.length}|${it.w ?? ""}|${it.trashed ?? ""}|${it.favorite ? 1 : 0}`;
const cardKind = () => (tab === "notes" ? "note" : tab === "trash" ? "trash" : "card");
function cardFor(it: Item, eager: boolean): HTMLElement {
  const k = `${cardKind()}:${it.id}`;
  const v = cardVer(it);
  const hit = cardCache.get(k);
  if (hit && hit.v === v) {
    hit.el.classList.toggle("sel", selected.has(it.id));
    revive(hit.el);
    return hit.el;
  }
  const c = cardKind() === "note" ? noteRow(it) : card(it, eager);
  c.dataset.kind = cardKind();
  if (hit) { const old = hit.el; if (old.isConnected) { c.style.cssText = old.style.cssText; old.replaceWith(c); } const p = posOf.get(old); if (p) posOf.set(c, p); }
  cardCache.set(k, { el: c, v });
  if (cardCache.size > 1500) { const oldest = cardCache.keys().next().value; if (oldest) cardCache.delete(oldest); }
  return c;
}

function card(it: Item, eager = false): HTMLElement {
  const c = el("div", "card tile" + (selected.has(it.id) ? " sel" : "") + (it.favorite && tab !== "trash" ? " fav" : ""));
  c.dataset.id = it.id;
  c.draggable = tab !== "trash";
  const media = isImg(it) || isVideo(it);
  if (tab !== "trash" && media) c.append(favBtn(it));
  // text cards: the star is one of the hover buttons, and a small gold star marks a favourite when not hovered
  if (tab !== "trash" && !media && it.favorite) { const m = el("span", "fmark"); m.innerHTML = ICON.star; c.append(m); } // static markup
  if (isImg(it)) {
    const img = el("img");
    img.loading = eager ? "eager" : "lazy";
    img.draggable = false;
    img.style.aspectRatio = it.w && it.h ? `${it.w} / ${it.h}` : "4 / 3";
    img.onload = () => {
      img.classList.add("ok");
      if (!it.w && img.naturalWidth) {
        // the sidecar had no size: take the thumbnail's and re-flow
        it.w = img.naturalWidth; it.h = img.naturalHeight;
        img.style.aspectRatio = `${it.w} / ${it.h}`;
        relayoutSoon();
      }
    };
    img.src = thumb(it);
    c.append(img);
  } else if (isVideo(it)) {
    const v = el("video");
    v.muted = true; v.loop = true; v.preload = "metadata"; v.draggable = false;
    v.style.aspectRatio = it.w && it.h ? `${it.w} / ${it.h}` : "16 / 9";
    v.src = full(it) + "#t=0.1";
    c.addEventListener("mouseenter", () => { v.play().catch(() => {}); });
    c.addEventListener("mouseleave", () => { v.pause(); v.currentTime = 0.1; });
    c.append(v, el("span", "vbadge", "▶"));
  } else if (it.kind === "text" || it.kind === "link" || isTextFile(it)) {
    const t = el("div", "t" + (it.kind === "link" ? " link" : ""), it.text ?? "");
    if (isTextFile(it)) { t.classList.add("code"); t.append(el("span", "badge", it.ext)); }
    c.append(t);
    if (isNote(it) && sourceLabel(it)) { const s = el("div", "src"); s.append(el("span", "", sourceLabel(it))); c.append(s); }
  } else {
    const fl = el("div", "f");
    fl.append(el("span", "badge", it.ext || "file"), el("span", "", it.original_path?.split(/[\\/]/).pop() ?? it.name));
    c.append(fl);
  }
  if (it.caption) c.append(el("div", "cap", it.caption));
  const acts = el("div", "acts");
  if (tab === "trash") {
    if (it.trashed) c.append(el("div", "ago", `Deleted ${ago(it.trashed)}${daysLeft(it)}`));
    acts.append(actBtn(ICON.restore, "Restore", () => restore([it.id])), actBtn(ICON.trash, "Delete forever", () => purge([it.id]), "d"));
  } else {
    if (!media) acts.append(actBtn(ICON.star, it.favorite ? "Remove from Favourites (F)" : "Add to Favourites (F)", () => setFav([it.id], !it.favorite), "fs" + (it.favorite ? " on" : "")));
    if (canCopy(it)) acts.append(actBtn(ICON.copy, "Copy", () => copyItem(it)));
    if (it.url) acts.append(actBtn(ICON.open, "Open source page", () => invoke("gallery_open_url", { url: it.url })));
    if (!it.favorite) acts.append(actBtn(ICON.trash, "Move to trash (with the rest of the selection when it is selected)", () => trash(selected.has(it.id) ? [...selected] : [it.id]), "d"));
  }
  c.append(acts);
  return c;
}

/** Notes tab card: the note itself, then where it came from (the page title, e.g. the YouTube video). */
function noteRow(it: Item): HTMLElement {
  const c = el("div", "card note tile" + (selected.has(it.id) ? " sel" : "") + (it.favorite ? " fav" : ""));
  c.dataset.id = it.id;
  c.draggable = true;
  c.append(favBtn(it));
  c.append(el("div", "nt" + (it.kind === "link" ? " link" : ""), it.caption && it.kind === "link" ? `${it.caption}\n${it.text ?? ""}` : it.text ?? it.caption));
  const s = el("div", "src");
  const label = sourceLabel(it);
  if (label) {
    if (it.url) {
      const ico = el("span"); ico.innerHTML = ICON.link; // static markup
      const a = el("a", "", label);
      a.title = it.url;
      a.addEventListener("click", (e) => { e.stopPropagation(); invoke("gallery_open_url", { url: it.url }); });
      s.append(ico, a);
    } else s.append(el("span", "", label));
  }
  for (const t of it.tags.slice(0, 4)) s.append(el("span", "tg", "#" + t));
  s.append(el("span", "tm", timeOf(it)));
  c.append(s);
  return c;
}

// ---------- layout: one continuous masonry with day labels, animated with FLIP ----------
// Every tile (cards and day labels) is absolutely positioned in #grid. A day label goes to the shortest column and
// its day's first item directly under it; the rest flow into whichever column is shortest, so short days share
// rows and short items stack instead of each day starting a new full-width row.
type Anim = "none" | "flip" | "switch" | "enter";
interface Pos { x: number; y: number; w: number }
const posOf = new WeakMap<HTMLElement, Pos>();
const labels = new Map<string, HTMLElement>();
let dayTops: { key: string; y: number }[] = [];
const liveCards = () => Array.from(grid.querySelectorAll<HTMLElement>(".card:not(.leaving)"));
const liveTiles = () => Array.from(grid.querySelectorAll<HTMLElement>(".card:not(.leaving), .dl:not(.leaving)"));
const tf = (p: { x: number; y: number }, dx = 0, dy = 0, s = 1) => `translate(${p.x + dx}px, ${p.y + dy}px)${s !== 1 ? ` scale(${s})` : ""}`;

function labelFor(key: string): HTMLElement {
  let l = labels.get(key);
  if (!l) {
    l = el("div", "dl");
    l.dataset.day = key;
    const b = el("button");
    b.title = "Show only this day";
    b.onclick = () => { f.day = f.day === key ? "" : key; renderRail(); filter("switch"); };
    l.append(b);
    labels.set(key, l);
  }
  revive(l);
  return l;
}

/** A tile that was leaving (or waiting to enter) comes back: drop its fade-out, which otherwise keeps filling
 *  "forwards" and hides the tile again once its entrance animation ends. */
const leaving = new WeakMap<HTMLElement, Animation>();
function revive(c: HTMLElement) {
  unburn(c);
  c.classList.remove("leaving");
  leaving.get(c)?.cancel();
  leaving.delete(c);
  c.style.opacity = "";
}
function leave(node: HTMLElement, animate: boolean, from: Pos | null) {
  node.classList.add("leaving");
  if (node.classList.contains("burned")) { node.remove(); return; } // already burnt away
  if (!animate) { node.remove(); return; }
  leaving.get(node)?.cancel();
  const a = from
    ? node.animate([{ opacity: 1, transform: tf(from) }, { opacity: 0, transform: tf(from, 0, 0, 0.86) }], { duration: 300, easing: MOVE_EASE, fill: "forwards" })
    : node.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 240, easing: MOVE_EASE, fill: "forwards" });
  leaving.set(node, a);
  a.onfinish = () => { if (node.classList.contains("leaving")) node.remove(); };
}

let layoutToken = 0;
/** Move / entrance animation running on a tile. A newer layout continues it from where it is on screen instead of
 *  letting it finish at the old spot and snap. */
const moving = new WeakMap<HTMLElement, Animation>();
function track(c: HTMLElement, a: Animation) {
  moving.set(c, a);
  a.addEventListener("finish", () => { if (moving.get(c) === a) moving.delete(c); });
}
function layout(anim: Anim) {
  const token = ++layoutToken;
  const animate = anim !== "none" && !reduced();
  const vh = wrap.clientHeight;
  // FIRST: where the tiles on screen are now (as drawn, mid-animation included) and how opaque they look
  const first = new Map<HTMLElement, { r: DOMRect; o: number; was: Pos | undefined }>();
  for (const c of liveTiles()) {
    const busy = moving.has(c);
    if (!busy && !(animate && anim !== "enter")) continue;
    const r = c.getBoundingClientRect();
    if (r.bottom > -100 && r.top < innerHeight + 100) first.set(c, { r, o: busy ? Number(getComputedStyle(c).opacity) : 1, was: posOf.get(c) && { ...posOf.get(c)! } });
  }

  const W = grid.clientWidth;
  layoutW = W;
  const GAP = view.gap;
  const minCol = tab === "notes" ? Math.round(Math.max(260, view.size * 1.55)) : view.size;
  const cols = Math.max(1, Math.floor((W + GAP) / (minCol + GAP)));
  const colW = Math.floor((W - GAP * (cols - 1)) / cols);

  // the sequence of tiles: a label before each day's first item (labels count the whole day)
  const totals = new Map<string, number>();
  for (const it of shown) totals.set(dayKey(it), (totals.get(dayKey(it)) ?? 0) + 1);
  const seq: { el: HTMLElement; label?: string; day?: string }[] = [];
  let lastDay = "", dayFirst = "";
  let eager = 0;
  for (const it of shown.slice(0, limit)) {
    const k = dayKey(it);
    if (k !== lastDay) {
      if (view.days) {
        const l = labelFor(k);
        l.firstElementChild!.replaceChildren(dayLabel(k), el("span", "", String(totals.get(k) ?? 0)));
        seq.push({ el: l, label: k });
      } else dayFirst = k; // no labels: the day starts at its first card (for the day pill and the scrubber)
      lastDay = k;
    }
    seq.push({ el: cardFor(it, eager++ < 40), day: dayFirst || undefined });
    dayFirst = "";
  }
  const want = new Set(seq.map((s) => s.el));
  for (const c of Array.from(grid.children) as HTMLElement[]) {
    if (c.classList.contains("leaving") || c.classList.contains("embers") || want.has(c)) continue;
    leave(c, animate && first.has(c), posOf.get(c) ?? null);
  }
  for (const s of seq) {
    if (s.el.parentElement !== grid) grid.append(s.el);
    if (s.el.style.width !== `${colW}px`) s.el.style.width = `${colW}px`;
  }
  // MEASURE (one reflow), then POSITION
  const hs = seq.map((s) => (s.label ? LABEL_H : s.el.offsetHeight));
  const colH = new Array(cols).fill(0);
  let forced = -1;
  dayTops = [];
  seq.forEach((s, i) => {
    let col = forced;
    if (col < 0) { col = 0; for (let j = 1; j < cols; j++) if (colH[j] < colH[col] - 1) col = j; }
    forced = s.label ? col : -1;
    const p = { x: col * (colW + GAP), y: colH[col], w: colW };
    colH[col] += hs[i] + (s.label ? 0 : GAP);
    posOf.set(s.el, p);
    s.el.style.transform = tf(p);
    if (s.label || s.day) dayTops.push({ key: (s.label || s.day)!, y: p.y });
  });
  grid.style.height = `${Math.max(0, Math.max(...colH) - GAP)}px`;
  syncDay();

  // a running animation whose target did not change keeps going untouched; any other is stopped here and
  // continued below from where the tile is on screen
  const same = (a: Pos | undefined, b: Pos) => !!a && a.x === b.x && a.y === b.y && a.w === b.w;
  for (const [c, f] of first) {
    if (moving.has(c) && same(f.was, posOf.get(c)!)) { first.delete(c); continue; }
    moving.get(c)?.cancel();
    moving.delete(c);
  }
  // LAST + PLAY: moved tiles glide from where they were (centre, scale and opacity); new ones cascade in
  const entering: { c: HTMLElement; r: DOMRect }[] = [];
  const wr = wrap.getBoundingClientRect();
  for (const { el: c } of seq) {
    const p = posOf.get(c)!;
    const f = first.get(c);
    if (f) {
      const r = c.getBoundingClientRect();
      const dx = f.r.left + f.r.width / 2 - (r.left + r.width / 2), dy = f.r.top + f.r.height / 2 - (r.top + r.height / 2);
      const k = r.width ? f.r.width / r.width : 1;
      if (Math.abs(dx) + Math.abs(dy) > 0.5 || Math.abs(k - 1) > 0.004 || f.o < 0.99)
        track(c, c.animate([{ transform: tf(p, dx, dy, k), opacity: f.o }, { transform: tf(p), opacity: 1 }], { duration: MOVE_MS, easing: MOVE_EASE }));
    } else if (animate && !moving.has(c)) {
      const r = c.getBoundingClientRect();
      if (r.bottom > wr.top && r.top < wr.top + vh + 40) entering.push({ c, r });
    }
  }
  if (!entering.length) return;
  const lefts = [...new Set(entering.map((e) => Math.round(e.r.left)))].sort((a, b) => a - b);
  const play = () => {
    // always undo the wait-for-images hiding, even if a newer layout already ran (a resize, an image size)
    for (const { c } of entering) c.style.opacity = "";
    if (token !== layoutToken) return;
    for (const { c, r } of entering) {
      const p = posOf.get(c)!;
      const d = anim === "enter"
        ? Math.min(600, 120 + ((r.top - wr.top) / 100) * 55 + lefts.indexOf(Math.round(r.left)) * 45)
        : Math.min(360, ((r.top - wr.top) / 100) * 26 + lefts.indexOf(Math.round(r.left)) * 34);
      track(c, c.animate([{ opacity: 0, transform: tf(p, 0, anim === "enter" ? 26 : 14, anim === "enter" ? 0.9 : 0.95) }, { opacity: 1, transform: tf(p) }],
        { duration: anim === "enter" ? 600 : 420, easing: SPRING, delay: d, fill: "backwards" }));
    }
  };
  // a switch waits a moment for the new pictures, so cards enter complete instead of as empty frames
  const waiting = anim === "switch" ? entering.map((e) => e.c.querySelector("img")).filter((im): im is HTMLImageElement => !!im && !im.complete) : [];
  if (!waiting.length) { play(); return; }
  for (const { c } of entering) c.style.opacity = "0";
  Promise.race([Promise.all(waiting.map((im) => im.decode().catch(() => {}))), new Promise((r) => setTimeout(r, 240))]).then(play);
}

let relayoutRaf = 0;
function relayoutSoon(anim: Anim = "none") {
  if (relayoutRaf) return;
  relayoutRaf = requestAnimationFrame(() => { relayoutRaf = 0; if (tab !== "canvases") layout(anim); });
}
// width changes (window resize, the detail panel opening) re-flow after they settle, with a glide
// While the window is still hidden (opening) a width change just lays out again; during the entrance it restarts
// the pop-in (the window settling to its size must never look like a re-arrange); later it glides.
let resizeTimer: number | undefined, layoutW = 0, enterUntil = 0;
new ResizeObserver(() => {
  const w = grid.clientWidth;
  if (!layoutW || w === layoutW || tab === "canvases") return;
  if (opening) { layout("none"); return; }
  if (performance.now() < enterUntil) { replayEnter(); return; }
  clearTimeout(resizeTimer);
  resizeTimer = window.setTimeout(() => layout("flip"), 90);
}).observe(wrap);
function replayEnter() {
  for (const c of liveTiles()) { moving.get(c)?.cancel(); moving.delete(c); c.style.opacity = ""; }
  enterUntil = performance.now() + 1400;
  layout("enter");
}

new IntersectionObserver((e) => {
  if (e[0].isIntersecting && limit < shown.length && tab !== "canvases") { limit += PAGE; layout("none"); renderScrub(); }
}, { root: wrap, rootMargin: "800px" }).observe($("sentinel"));

/** The day you are scrolled to, floating at the top of the grid. */
function currentDay(): string {
  const top = wrap.scrollTop - grid.offsetTop + 20;
  let cur = dayTops[0]?.key ?? "";
  for (const d of dayTops) if (d.y <= top) cur = d.key;
  return cur;
}
function syncDay() {
  const on = wrap.scrollTop > 60 && dayTops.length > 0 && tab !== "canvases";
  daypill.classList.toggle("on", on);
  if (on) daypill.textContent = dayLabel(currentDay());
  syncScrubPos();
}

// ---------- smooth scrolling ----------
// Mouse wheels move in coarse notches; ease each notch over ~200 ms. Precision touchpads (small pixel deltas)
// keep native scrolling. The Animations setting / reduced motion turn it off.
function smoothScroller(box: HTMLElement) {
  let target = 0, pos = 0, raf = 0, last = 0;
  const step = (t: number) => {
    const dt = last ? Math.min(50, t - last) : 16;
    last = t;
    if (Math.abs(box.scrollTop - Math.round(pos)) > 2) { stop(); return; } // the user dragged the scrollbar
    pos += (target - pos) * (1 - Math.exp(-dt / 65));
    if (Math.abs(target - pos) < 0.5) pos = target;
    box.scrollTop = pos;
    if (pos !== target) raf = requestAnimationFrame(step); else stop();
  };
  const stop = () => { if (raf) cancelAnimationFrame(raf); raf = 0; last = 0; };
  box.addEventListener("wheel", (e) => {
    if (e.ctrlKey || e.shiftKey || reduced()) return;
    const px = e.deltaMode === 1 ? e.deltaY * 40 : e.deltaMode === 2 ? e.deltaY * box.clientHeight : e.deltaY;
    if (e.deltaMode === 0 && Math.abs(px) < 40 && !raf) return; // touchpad: native
    e.preventDefault();
    if (!raf) { pos = box.scrollTop; target = pos; }
    target = Math.max(0, Math.min(box.scrollHeight - box.clientHeight, target + px));
    if (!raf) raf = requestAnimationFrame(step);
  }, { passive: false });
  return { stop };
}
const smooth = smoothScroller(wrap);
smoothScroller(rail);
smoothScroller($("detail-in"));
smoothScroller(cvhome);

// ---------- timeline scrubber ----------
let scrubDays: { key: string; at: number; n: number }[] = [];
function renderScrub() {
  scrub.replaceChildren();
  const days = new Map<string, number>();
  for (const it of shown) days.set(dayKey(it), (days.get(dayKey(it)) ?? 0) + 1);
  scrub.classList.toggle("off", tab === "canvases" || days.size < 3);
  if (days.size < 3) return;
  // positions follow item counts, so busy days get more room
  let acc = 0;
  scrubDays = [...days.entries()].map(([key, n]) => { const d = { key, at: acc / shown.length, n }; acc += n; return d; });
  let lastMonth = "", lastAt = -1;
  for (const d of scrubDays) {
    const month = d.key.slice(0, 7);
    const tk = el("i", "tk" + (month !== lastMonth ? " m" : ""));
    tk.style.top = `${d.at * 100}%`;
    scrub.append(tk);
    if (month !== lastMonth && d.at - lastAt > 0.06) {
      const [y, m] = month.split("-").map(Number);
      const ml = el("span", "ml", new Date(y, m - 1, 1).toLocaleDateString(undefined, { month: "short", year: y === new Date().getFullYear() ? undefined : "2-digit" }));
      ml.style.top = `${d.at * 100}%`;
      scrub.append(ml);
      lastAt = d.at;
    }
    lastMonth = month;
  }
  scrub.append(el("span", "bub"), el("i", "pos"));
  syncScrubPos();
}
function dayAtFrac(fr: number) {
  let best = scrubDays[0];
  for (const d of scrubDays) if (d.at <= fr) best = d;
  return best;
}
function jumpToDay(key: string) {
  const idx = shown.findIndex((it) => dayKey(it) === key);
  if (idx < 0) return;
  if (idx >= limit - 20) { limit = Math.min(shown.length, idx + PAGE); layout("none"); }
  const d = dayTops.find((t) => t.key === key);
  if (d) { smooth.stop(); wrap.scrollTop = d.y + grid.offsetTop - 8; }
}
function syncScrubPos() {
  const pos = scrub.querySelector<HTMLElement>(".pos");
  if (!pos || !scrubDays.length) return;
  const cur = scrubDays.find((x) => x.key === currentDay()) ?? scrubDays[0];
  pos.style.top = `${cur.at * 100}%`;
}
let dayRaf = 0;
wrap.addEventListener("scroll", () => { if (!dayRaf) dayRaf = requestAnimationFrame(() => { dayRaf = 0; syncDay(); }); }, { passive: true });
scrub.addEventListener("pointermove", (e) => {
  const r = scrub.getBoundingClientRect();
  const fr = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
  const d = dayAtFrac(fr);
  const bub = scrub.querySelector<HTMLElement>(".bub");
  if (d && bub) { bub.textContent = dayLabel(d.key); bub.style.top = `${fr * 100}%`; }
  if (scrub.classList.contains("drag") && d) jumpToDay(d.key);
});
scrub.addEventListener("pointerdown", (e) => {
  scrub.setPointerCapture(e.pointerId);
  scrub.classList.add("drag");
  const r = scrub.getBoundingClientRect();
  const d = dayAtFrac(Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)));
  if (d) jumpToDay(d.key);
  const up = () => scrub.classList.remove("drag");
  scrub.addEventListener("pointerup", up, { once: true });
  scrub.addEventListener("pointercancel", up, { once: true });
});

// ---------- selection ----------
function markSel() {
  document.querySelectorAll<HTMLElement>(".card, .cv-item").forEach((c) => c.classList.toggle("sel", selected.has(c.dataset.id!)));
  updateSelBar();
}
function refreshSel() {
  markSel();
  showDetail();
}

const selbar = el("div");
selbar.id = "selbar";
document.body.append(selbar);
function updateSelBar() {
  document.body.classList.toggle("multi", selected.size > 0);
  selbar.replaceChildren();
  selbar.classList.toggle("on", selected.size > 0);
  if (!selected.size) { pop.classList.remove("on"); return; }
  const ibtn = (icon: string, title: string, fn: () => void, cls = "") => { const b = el("button", "btn ib " + cls); b.innerHTML = icon; b.title = title; b.setAttribute("aria-label", title); b.onclick = fn; return b; }; // static markup
  const ids = () => [...selected];
  const selectAll = () => { selected = new Set(shown.map((i) => i.id)); refreshSel(); };
  const clear = ibtn(ICON.close, "Clear the selection (Esc)", () => { selected.clear(); refreshSel(); });
  selbar.append(el("span", "n", `${selected.size} selected`));
  if (tab === "trash") {
    selbar.append(ibtn(ICON.restore, "Restore", () => restore(ids())), ibtn(ICON.selAll, "Select all (Ctrl+A)", selectAll), ibtn(ICON.trash, "Delete forever", () => purge(ids()), "danger"), clear);
  } else if (tab === "canvases") {
    selbar.append(ibtn(ICON.trash, "Remove from this canvas (Delete; they stay in your library)", () => removeFromCanvas(ids()), "danger"), clear);
  } else {
    const create = el("button", "btn glow");
    create.innerHTML = ICON.canvas; // static markup
    create.append("Create canvas");
    create.title = "Lay the selected items out on a new canvas (saved in the Canvases tab)";
    create.onclick = () => createCanvas(ids());
    selbar.append(create);
    const allFav = [...selected].every((id) => byId(id)?.favorite);
    selbar.append(ibtn(ICON.star, allFav ? "Remove from Favourites (F)" : "Add to Favourites (F)", () => toggleFavSel(), allFav ? "favon" : ""),
      ibtn(ICON.tag, "Tag… (T)", () => openTagPop()), ibtn(ICON.selAll, "Select all (Ctrl+A)", selectAll),
      ibtn(ICON.exp, "Export…", () => exportItems(ids())));
    if (!allFav) selbar.append(ibtn(ICON.trash, "Move to trash (Delete; favourites are kept)", () => trash(ids()), "danger"));
    selbar.append(clear);
  }
}

// Rubber-band selection: drag on empty space; Ctrl/Shift adds to the current selection.
const marq = el("div");
marq.id = "marq";
document.body.append(marq);
wrap.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || (e.target as HTMLElement).closest(".card, .dl button")) return;
  if (e.clientX >= wrap.getBoundingClientRect().left + wrap.clientWidth) return; // scrollbar
  const [x0, y0] = [e.clientX, e.clientY];
  const base = e.ctrlKey || e.shiftKey ? new Set(selected) : new Set<string>();
  let active = false;
  const move = (m: PointerEvent) => {
    if (!active && Math.hypot(m.clientX - x0, m.clientY - y0) < 5) return;
    active = true;
    const [l, t, r, b] = [Math.min(x0, m.clientX), Math.min(y0, m.clientY), Math.max(x0, m.clientX), Math.max(y0, m.clientY)];
    Object.assign(marq.style, { display: "block", left: l + "px", top: t + "px", width: r - l + "px", height: b - t + "px" });
    const hits = new Set(base);
    liveCards().forEach((c) => {
      const q = c.getBoundingClientRect();
      if (q.left < r && q.right > l && q.top < b && q.bottom > t) hits.add(c.dataset.id!);
    });
    selected = hits;
    markSel();
    const wr = wrap.getBoundingClientRect();
    if (m.clientY > wr.bottom - 30) wrap.scrollTop += 18; else if (m.clientY < wr.top + 30) wrap.scrollTop -= 18;
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    marq.style.display = "none";
    if (!active && !base.size) selected.clear();
    refreshSel();
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up, { once: true });
});

function clickSelect(id: string, e: MouseEvent, list: Item[] = shown) {
  const idx = list.findIndex((i) => i.id === id);
  if (e.shiftKey && anchor >= 0) {
    const [a, b] = [Math.min(anchor, idx), Math.max(anchor, idx)];
    selected = new Set(list.slice(a, b + 1).map((i) => i.id));
  } else if (e.ctrlKey || e.metaKey) {
    selected.has(id) ? selected.delete(id) : selected.add(id);
    anchor = idx;
  } else {
    selected = new Set([id]);
    anchor = idx;
  }
  refreshSel();
}
grid.addEventListener("click", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (c) clickSelect(c.dataset.id!, e);
});
grid.addEventListener("dblclick", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (c) openViewer(shown.findIndex((i) => i.id === c.dataset.id));
});

/** Arrow keys: left/right step through the list, up/down go to the nearest card above/below. */
function moveSel(key: string) {
  const cur = selected.size === 1 ? [...selected][0] : null;
  const idx = cur ? shown.findIndex((i) => i.id === cur) : -1;
  let next = -1;
  if (idx < 0) next = 0;
  else if (key === "ArrowRight") next = Math.min(shown.length - 1, idx + 1);
  else if (key === "ArrowLeft") next = Math.max(0, idx - 1);
  else {
    const from = document.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(cur!)}"]:not(.leaving)`)?.getBoundingClientRect();
    if (!from) next = idx;
    else {
      const down = key === "ArrowDown";
      let best: HTMLElement | null = null, bd = Infinity;
      for (const c of liveCards()) {
        const r = c.getBoundingClientRect();
        const dy = down ? r.top - from.bottom : from.top - r.bottom;
        if (dy < -4) continue;
        const d = dy + Math.abs(r.left + r.width / 2 - (from.left + from.width / 2)) * 2;
        if (d < bd && c.dataset.id !== cur) { bd = d; best = c; }
      }
      next = best ? shown.findIndex((i) => i.id === best!.dataset.id) : idx;
    }
  }
  if (next < 0 || !shown[next]) return;
  if (next >= limit) { limit = next + PAGE; layout("none"); }
  selected = new Set([shown[next].id]);
  anchor = next;
  refreshSel();
  document.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(shown[next].id)}"]:not(.leaving)`)?.scrollIntoView({ block: "nearest", behavior: reduced() ? "auto" : "smooth" });
}

/** Native file drag of library items (the files themselves, so Explorer, Figma, Notion... get the real file). */
async function nativeDrag(ids: string[]) {
  const picked = ids.map((i) => byId(i)).filter((i): i is Item => !!i && !i.trashed);
  if (!picked.length) return;
  dragging = true;
  try {
    const icon = await invoke<string>("gallery_drag_icon", { id: picked[0].id }).catch(() => picked[0].icon);
    await startDrag({ item: picked.map((p) => p.path), icon });
  } catch (err) {
    console.error(err);
  } finally {
    setTimeout(() => (dragging = false), 400);
  }
}
/** Makes a preview element (viewer picture, panel preview) drag its item's file instead of the picture. */
function dragsFile(media: HTMLElement, id: string) {
  media.draggable = true;
  media.addEventListener("dragstart", (e) => { e.preventDefault(); e.stopPropagation(); nativeDrag([id]); });
}
grid.addEventListener("dragstart", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card");
  if (!c || tab === "trash") return;
  e.preventDefault(); // replace the HTML5 drag with a native file drag
  const id = c.dataset.id!;
  nativeDrag(selected.has(id) ? [...selected] : [id]);
});

// ---------- viewer: zooms open from its card ----------
let viewList: Item[] = [];
function openViewer(i: number, list: Item[] = tab === "canvases" ? cv.list : shown) {
  if (i < 0 || i >= list.length) return;
  viewList = list;
  const wasOpen = viewer.classList.contains("on");
  viewIdx = i;
  const it = list[i];
  viewer.replaceChildren();
  let media: HTMLElement | null = null;
  if (isImg(it)) {
    const img = el("img");
    img.src = thumb(it); // already decoded in the grid: the zoom starts from a real picture
    if (ext(it) === "png" || ext(it) === "jpg" || ext(it) === "jpeg") {
      const hi = new Image();
      hi.onload = () => { if (viewIdx === i) img.src = hi.src; };
      hi.src = full(it);
    }
    if (it.w && it.h) img.style.aspectRatio = `${it.w} / ${it.h}`;
    if (!it.trashed) dragsFile(img, it.id);
    img.style.width = it.w && it.h ? `min(92vw, ${it.w}px, calc(84vh * ${it.w / it.h}))` : "";
    viewer.append(img);
    media = img;
  } else if (isVideo(it)) {
    const v = el("video");
    v.src = full(it); v.controls = true; v.autoplay = true; v.loop = true;
    v.addEventListener("click", (e) => e.stopPropagation());
    viewer.append(v);
  } else if (isAudio(it)) {
    const box = el("div", "vt aud", it.original_path?.split(/[\\/]/).pop() ?? it.name);
    const a = el("audio");
    a.src = full(it); a.controls = true; a.autoplay = true;
    a.addEventListener("click", (e) => e.stopPropagation());
    box.append(a);
    viewer.append(box);
  } else if (isPdf(it)) {
    const fr = el("iframe");
    fr.src = full(it);
    fr.addEventListener("click", (e) => e.stopPropagation());
    viewer.append(fr);
  } else if (isTextFile(it)) {
    const pre = el("div", "vt code", "Loading…");
    pre.addEventListener("click", (e) => e.stopPropagation());
    viewer.append(pre);
    fetch(full(it)).then((r) => r.text()).then((t) => { if (viewIdx === i) pre.textContent = t.length > 300000 ? t.slice(0, 300000) + "\n\n… (truncated)" : t; }).catch(() => (pre.textContent = it.text ?? ""));
  } else {
    const t = el("div", "vt", it.text ?? it.name);
    viewer.append(t);
    media = t;
  }
  if (it.caption) viewer.append(el("div", "vc", it.caption));
  for (const [dir, label, side] of [[-1, "‹", "left"], [1, "›", "right"]] as const) {
    const b = el("button", "vn", label);
    b.style[side] = "22px";
    b.onclick = (e) => { e.stopPropagation(); stepViewer(dir); };
    viewer.append(b);
  }
  viewer.classList.add("on");
  // zoom from the card (FLIP on the media element)
  const src = document.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(it.id)}"]:not(.leaving), .cv-item[data-id="${CSS.escape(it.id)}"]`);
  if (!wasOpen && media && src && !reduced()) {
    const a = src.getBoundingClientRect(), b = media.getBoundingClientRect();
    if (a.width && b.width) {
      const sx = a.width / b.width, sy = a.height / b.height;
      media.animate([
        { transformOrigin: "0 0", transform: `translate(${a.left - b.left}px, ${a.top - b.top}px) scale(${sx}, ${sy})`, borderRadius: "12px" },
        { transformOrigin: "0 0", transform: "none" },
      ], { duration: 240, easing: EASE });
    }
  }
}
function closeViewer() {
  viewer.classList.remove("on");
  viewIdx = -1;
}
viewer.addEventListener("click", closeViewer);

// ---------- tag chip input with autocomplete ----------
function allTags(): [string, number][] {
  const m = new Map<string, [string, number]>();
  for (const it of items) for (const t of it.tags) { const k = t.toLowerCase(); const e = m.get(k); m.set(k, [e?.[0] ?? t, (e?.[1] ?? 0) + 1]); }
  return [...m.values()].sort((a, b) => b[1] - a[1]);
}
/** Chips for `tags` plus a text field: Enter/comma/Tab adds (or picks the highlighted suggestion), Backspace on an
 *  empty field removes the last chip, ↑/↓ move through suggestions. */
function tagInput(tags: string[], onAdd: (t: string) => void, onRemove: (t: string) => void, placeholder = "Add a tag…"): HTMLElement {
  const box = el("div", "tagin");
  const inp = el("input");
  inp.placeholder = placeholder;
  inp.spellcheck = false;
  const sugg = el("div", "sugg");
  let hi = -1;
  let cur = [...tags];
  const chips = () => {
    box.querySelectorAll(".tc").forEach((c) => c.remove());
    for (const t of cur) {
      const c = el("span", "tc", "#" + t);
      const x = el("button", "", "×");
      x.title = `Remove #${t}`;
      x.onclick = (e) => { e.stopPropagation(); cur = cur.filter((y) => y !== t); chips(); onRemove(t); };
      c.append(x);
      box.insertBefore(c, inp);
    }
  };
  const options = () => {
    const v = inp.value.trim().replace(/^#/, "").toLowerCase();
    return allTags().filter(([t]) => !cur.some((c) => c.toLowerCase() === t.toLowerCase()) && (!v || t.toLowerCase().includes(v))).slice(0, 6);
  };
  const paint = () => {
    const o = options();
    sugg.replaceChildren(...o.map(([t, n], i) => {
      const b = el("button", i === hi ? "hi" : "", "#" + t);
      b.append(el("span", "", String(n)));
      b.onmousedown = (e) => { e.preventDefault(); add(t); };
      return b;
    }));
    sugg.classList.toggle("on", document.activeElement === inp && o.length > 0);
  };
  const add = (raw: string) => {
    const t = raw.trim().replace(/^#+/, "").trim();
    inp.value = "";
    hi = -1;
    if (t && !cur.some((c) => c.toLowerCase() === t.toLowerCase())) { cur.push(t); chips(); onAdd(t); }
    paint();
  };
  inp.addEventListener("input", () => {
    if (/[,\s]$/.test(inp.value) && inp.value.trim()) { add(inp.value.replace(/[,\s]+$/, "")); return; }
    hi = -1;
    paint();
  });
  inp.addEventListener("keydown", (e) => {
    const o = options();
    if (e.key === "ArrowDown" && o.length) { e.preventDefault(); hi = (hi + 1) % o.length; paint(); }
    else if (e.key === "ArrowUp" && o.length) { e.preventDefault(); hi = (hi - 1 + o.length) % o.length; paint(); }
    else if (e.key === "Enter" || (e.key === "Tab" && (inp.value.trim() || hi >= 0))) {
      e.preventDefault();
      if (hi >= 0 && o[hi]) add(o[hi][0]); else if (inp.value.trim()) add(inp.value); else inp.blur();
    } else if (e.key === "Backspace" && !inp.value && cur.length) { const t = cur.pop()!; chips(); onRemove(t); paint(); }
    else if (e.key === "Escape") { e.stopPropagation(); inp.value = ""; inp.blur(); }
    e.stopPropagation(); // gallery shortcuts stay off while typing
  });
  inp.addEventListener("focus", paint);
  inp.addEventListener("blur", () => { sugg.classList.remove("on"); if (inp.value.trim()) add(inp.value); });
  box.addEventListener("click", () => inp.focus());
  box.append(inp, sugg);
  chips();
  return box;
}

/** Re-renders an item's card in place after an edit, keeping its position. */
function refreshItem(it: Item) {
  for (const kind of ["card", "note", "trash"]) {
    const hit = cardCache.get(`${kind}:${it.id}`);
    if (!hit) continue;
    cardCache.delete(`${kind}:${it.id}`);
    if (hit.el.isConnected && kind === cardKind()) {
      const c = cardFor(it, true);
      c.style.cssText = hit.el.style.cssText;
      const p = posOf.get(hit.el);
      if (p) posOf.set(c, p);
      hit.el.replaceWith(c);
    }
  }
  relayoutSoon();
}

async function saveMeta(it: Item, caption: string, tags: string[]) {
  await invoke("gallery_set_meta", { id: it.id, caption, tags });
  it.caption = caption.trim();
  it.tags = tags;
  refreshItem(it);
  renderRail();
}

// ---------- detail panel ----------
function fmtBytes(n: number) { return n > 1e6 ? (n / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(n / 1e3)) + " KB"; }

// The panel floats over the grid, see-through and click-through (only its fields take the pointer), so whatever is
// under it can still be seen and selected; it fades further while the pointer is over something underneath.
// Actions live in the selection bar, not here.
const detailBox = $("detail");
addEventListener("pointermove", (e) => {
  if (!document.body.classList.contains("detail")) return;
  const r = detailBox.getBoundingClientRect();
  const over = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
  detailBox.classList.toggle("peek", over && !detailBox.contains(e.target as Node));
}, { passive: true });
// The panel is click-through, so the wheel would scroll the grid under it: over the panel it scrolls the panel
// while the panel has more to show in that direction.
addEventListener("wheel", (e) => {
  if (!document.body.classList.contains("detail") || e.ctrlKey) return;
  const r = detailBox.getBoundingClientRect();
  if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) return;
  const can = e.deltaY > 0 ? detail.scrollTop + detail.clientHeight < detail.scrollHeight - 1 : detail.scrollTop > 0;
  if (!can) return;
  e.preventDefault();
  e.stopPropagation();
  detail.scrollTop += e.deltaMode === 1 ? e.deltaY * 40 : e.deltaY;
}, { passive: false, capture: true });
function showDetail() {
  detail.replaceChildren();
  detail.classList.remove("fresh");
  void detail.offsetWidth;
  detail.classList.add("fresh");
  const sel = [...selected].map((id) => byId(id)!).filter(Boolean);
  document.body.classList.toggle("detail", sel.length > 0);
  if (!sel.length) return;
  if (sel.length > 1) {
    detail.append(el("h3", "", `${sel.length} selected`));
    if (tab !== "trash") detail.append(el("label", "l", "Tags on all of them"), multiTagInput(sel));
    return;
  }
  const it = sel[0];
  if (isImg(it)) {
    const pv = el("img", "pv");
    dragsFile(pv, it.id);
    pv.src = ext(it) === "png" ? full(it) : thumb(it);
    pv.onclick = () => openViewer(shown.findIndex((x) => x.id === it.id));
    detail.append(pv);
  } else if (it.text) {
    detail.append(el("div", "tx", it.text));
  }
  if (tab === "trash") {
    if (it.trashed) detail.append(el("div", "meta", `Deleted ${ago(it.trashed)}${daysLeft(it)}`));
    return;
  }

  const cap = el("input", "field"); cap.type = "text"; cap.value = it.caption; cap.placeholder = "Add a caption";
  cap.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); cap.blur(); }
    else if (e.key === "Escape") { cap.value = it.caption; cap.blur(); e.stopPropagation(); }
  });
  cap.addEventListener("blur", () => { if (cap.value.trim() !== it.caption) saveMeta(it, cap.value, it.tags); });
  const tags = tagInput(it.tags,
    (t) => saveMeta(it, it.caption, [...it.tags, t]),
    (t) => saveMeta(it, it.caption, it.tags.filter((x) => x !== t)));
  detail.append(el("label", "l", "Caption"), cap, el("label", "l", "Tags"), tags);
  if (it.palette.length) {
    const pal = el("div", "pal");
    for (const col of it.palette) {
      const b = el("button", "dot");
      b.style.background = col;
      b.title = `${col} — click to filter by this colour`;
      b.onclick = () => { f.color = bucketOf(col); renderRail(); filter("switch"); };
      pal.append(b);
    }
    detail.append(el("label", "l", "Palette"), pal);
  }
  if (it.ocr) {
    const d = el("details", "ocr");
    d.append(el("summary", "", "Text found in image"), el("div", "", it.ocr));
    detail.append(d);
  }

  const meta = el("div", "meta");
  const add = (k: string, v: string | HTMLElement) => { const b = el("b"); typeof v === "string" ? (b.textContent = v) : b.append(v); meta.append(el("span", "", k), b); };
  add("Saved", new Date(it.created).toLocaleString());
  add("How", it.method);
  if (it.w && it.h) add("Size", `${it.w} × ${it.h}`);
  const file = el("a", "", `${it.ext.toUpperCase()} · ${fmtBytes(it.bytes)}`);
  file.title = "Show in folder";
  file.onclick = () => invoke("gallery_reveal", { id: it.id });
  add("File", file);
  if (it.process) add("App", it.process);
  if (it.source_title) add("Window", it.source_title);
  if (it.url) { const a = el("a", "", it.url); a.onclick = () => invoke("gallery_open_url", { url: it.url }); add("URL", a); }
  if (it.original_path) add("From", it.original_path);
  detail.append(meta);
}

/** Tags shared by every selected item: removing one removes it from all, adding adds to all. */
function multiTagInput(sel: Item[]): HTMLElement {
  const common = sel[0].tags.filter((t) => sel.every((s) => hasTag(s, t)));
  const apply = async (add: string[], remove: string[]) => {
    const ids = sel.map((s) => s.id);
    await invoke("gallery_tag", { ids, add, remove });
    for (const s of sel) {
      s.tags = s.tags.filter((t) => !remove.some((r) => r.toLowerCase() === t.toLowerCase()));
      for (const a of add) if (!hasTag(s, a)) s.tags.push(a);
      refreshItem(s);
    }
    renderRail();
    flash(add.length ? `Tagged ${sel.length} with #${add[0]}` : `Removed #${remove[0]} from ${sel.length}`);
  };
  return tagInput(common, (t) => apply([t], []), (t) => apply([], [t]), "Tag all selected…");
}

function openTagPop() {
  const sel = [...selected].map((id) => byId(id)!).filter(Boolean);
  if (!sel.length) return;
  pop.replaceChildren(el("h4", "", `Tag ${sel.length} item${sel.length === 1 ? "" : "s"}`), multiTagInput(sel));
  pop.classList.add("on");
  pop.querySelector("input")?.focus();
}
document.addEventListener("pointerdown", (e) => {
  if (pop.classList.contains("on") && !(e.target as HTMLElement).closest("#tagpop, #selbar")) pop.classList.remove("on");
});

// ---------- export ----------
async function exportItems(ids?: string[]) {
  const list = ids ?? (selected.size ? [...selected] : shown.map((i) => i.id));
  if (!list.length) { flash("Nothing to export"); return; }
  const dest = await pickFolder({ directory: true, title: `Export ${list.length} item${list.length === 1 ? "" : "s"} to…` });
  if (typeof dest !== "string") return;
  try {
    const out = await invoke<string>("gallery_export", { ids: list, dest });
    flash(`Exported ${list.length} item${list.length === 1 ? "" : "s"}`, { label: "Open folder", fn: () => invoke("gallery_open_path", { path: out }) });
  } catch (e) {
    flash(String(e));
  }
}
$("export").addEventListener("click", () => exportItems());

// ---------- viewer stepping ----------
function stepViewer(dir: number) {
  openViewer(viewIdx + dir, viewList);
}

// ---------- canvases ----------
// A canvas is created from a selection ("Create canvas") and saved under its own name in the Canvases tab. It is
// auto-arranged once (notes as sticky notes, images at their real aspect ratio, clustered by source page), then
// items can be moved, panned and zoomed; every change saves itself and the header says so.
const CANVAS_MAX = 500;
const cv = { doc: null as CanvasDoc | null, built: "", size: new Map<string, [number, number]>(), list: [] as Item[], pan: [40, 40] as [number, number], zoom: 1 };
const cvBar = $("cv-bar"), cvNote = $("cv-note");
const cvName = el("input");
const cvStatus = el("span", "st", "Saved");

function newCanvasId() {
  const d = new Date();
  return `cv-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}-${Math.random().toString(16).slice(2, 6)}`;
}

async function createCanvas(ids: string[]) {
  if (!ids.length) return;
  const order = shown.map((i) => i.id).filter((id) => ids.includes(id));
  for (const id of ids) if (!order.includes(id)) order.push(id);
  const now = new Date().toISOString();
  const nth = canvases.length + 1;
  const doc: CanvasDoc = { v: 2, id: newCanvasId(), name: `Canvas ${nth} · ${new Date().toLocaleDateString(undefined, { day: "numeric", month: "short" })}`, created: now, modified: now, ids: order, pos: {}, labels: [], pan: null, zoom: 1 };
  try {
    doc.modified = await invoke<string>("canvas_save", { id: doc.id, data: doc });
  } catch (e) { flash(String(e)); return; }
  await loadCanvases();
  selected.clear();
  cv.doc = doc;
  store.set("stash.gallery.canvas", doc.id);
  setTab("canvases");
  moveTabInd();
  flash(`Created “${doc.name}”. It is saved in the Canvases tab.`);
}


/** The Canvases tab home: every saved canvas with a small collage. */
function renderCanvases() {
  cv.doc = null;
  canvasEl.classList.remove("on");
  cvhome.classList.add("on");
  cvhome.replaceChildren();
  renderRail();
  if (!canvases.length) {
    const e = el("div", "cvempty");
    e.append(el("h2", "", "No canvases yet"), el("div", "", "Select items in Notes, Visuals or All, then press Create canvas. Every canvas is saved here."));
    cvhome.append(e);
    return;
  }
  const list = el("div", "cvs");
  for (const c of canvases) {
    const card = el("div", "cvc");
    const col = el("div", "col4");
    for (const id of c.ids.slice(0, 4)) {
      const it = items.find((i) => i.id === id);
      const t = el("div");
      if (it && isImg(it)) t.style.backgroundImage = `url("${thumb(it)}")`;
      else if (it) t.textContent = (it.text ?? it.caption ?? it.name).slice(0, 90);
      col.append(t);
    }
    const del = actBtn(ICON.trash, "Delete canvas (the items stay in your library)", async () => {
      const doc = await invoke<CanvasDoc>("canvas_load", { id: c.id }).catch(() => null);
      await burnAll([card]);
      await invoke("canvas_delete", { id: c.id });
      await loadCanvases();
      if (store.get("stash.gallery.canvas") === c.id) store.set("stash.gallery.canvas", "");
      renderCanvases();
      flash(`Deleted “${c.name}”`, doc ? { label: "Undo", fn: async () => { await invoke("canvas_save", { id: doc.id, data: doc }); await loadCanvases(); renderCanvases(); } } : undefined);
    }, "d del");
    card.append(col, el("div", "nm", c.name || "Untitled canvas"), el("div", "mt", `${c.ids.length} item${c.ids.length === 1 ? "" : "s"} · edited ${ago(c.modified)}`), del);
    card.onclick = () => openCanvasDoc(c.id);
    list.append(card);
  }
  cvhome.append(list);
  count.textContent = `${canvases.length} canvas${canvases.length === 1 ? "" : "es"}`;
}

function closeCanvasDoc() {
  cvAdd.classList.remove("on");
  flushCanvas();
  store.set("stash.gallery.canvas", "");
  selected.clear();
  updateSelBar();
  showDetail();
  loadCanvases().then(renderCanvases);
}

async function openCanvasDoc(id: string) {
  const doc = cv.doc?.id === id ? cv.doc : await invoke<CanvasDoc>("canvas_load", { id }).catch(() => null);
  if (!doc) { flash("That canvas could not be opened"); renderCanvases(); return; }
  doc.pos ??= {}; doc.ids ??= [];
  doc.labels = [];
  cv.doc = doc;
  store.set("stash.gallery.canvas", id);
  selected.clear();
  updateSelBar();
  showDetail();
  showCanvasEditor(false);
}

function showCanvasEditor(keepView: boolean) {
  const doc = cv.doc!;
  cvhome.classList.remove("on");
  canvasEl.classList.add("on");
  renderRail();
  cvName.value = doc.name;
  setStatus("saved");
  buildCanvas(keepView && cv.built === doc.id); // keep the view only when returning to the canvas already on screen
}

let saveState: "saved" | "busy" | "err" = "saved";
function setStatus(s: typeof saveState) {
  saveState = s;
  cvStatus.className = "st" + (s === "busy" ? " busy" : s === "err" ? " err" : "");
  cvStatus.textContent = s === "busy" ? "Saving…" : s === "err" ? "Not saved, retrying" : "All changes saved";
  cvStatus.title = cv.doc ? `Saved in the Canvases tab · last saved ${new Date(cv.doc.modified).toLocaleTimeString()}` : "";
}

let cvSaveTimer: number | undefined;
function cvSave() {
  if (!cv.doc) return;
  setStatus("busy");
  clearTimeout(cvSaveTimer);
  cvSaveTimer = window.setTimeout(flushCanvas, 350);
}
async function flushCanvas() {
  clearTimeout(cvSaveTimer);
  const doc = cv.doc;
  if (!doc || saveState === "saved") return;
  doc.pan = cv.pan;
  doc.zoom = cv.zoom;
  try {
    doc.modified = await invoke<string>("canvas_save", { id: doc.id, data: doc });
    if (cv.doc === doc) setStatus("saved");
    const c = canvases.find((x) => x.id === doc.id);
    if (c) { c.modified = doc.modified; c.name = doc.name; c.ids = doc.ids; }
  } catch {
    if (cv.doc === doc) { setStatus("err"); cvSaveTimer = window.setTimeout(flushCanvas, 2000); }
  }
}
window.addEventListener("beforeunload", () => { if (cv.doc && saveState !== "saved") invoke("canvas_save", { id: cv.doc.id, data: cv.doc }); });

function cvApply() {
  world.style.transform = `translate(${cv.pan[0]}px, ${cv.pan[1]}px) scale(${cv.zoom})`;
  const z = cvBar.querySelector(".z");
  if (z) z.textContent = `${Math.round(cv.zoom * 100)}%`;
}

function cvItem(it: Item): HTMLElement {
  const d = el("div", "cv-item" + (selected.has(it.id) ? " sel" : ""));
  d.dataset.id = it.id;
  if (isImg(it)) {
    const img = el("img");
    img.draggable = false;
    img.src = thumb(it);
    d.append(img);
  } else if (isVideo(it)) {
    const v = el("video");
    v.muted = true; v.preload = "metadata";
    v.src = full(it) + "#t=0.1";
    d.append(v);
  } else if (isNote(it) || isTextFile(it)) {
    d.classList.add("cv-note");
    d.append(el("div", "nt", it.text ?? it.caption));
    const src = sourceLabel(it);
    if (src) d.append(el("div", "src", src));
  } else {
    d.classList.add("cv-file");
    d.append(el("span", "badge", it.ext || "file"), el("span", "", it.original_path?.split(/[\\/]/).pop() ?? it.name));
  }
  return d;
}

/** Cluster key: the source page (URL without its fragment), else the window title, else the app. */
function clusterOf(it: Item): [string, string] {
  if (it.url) return [it.url.replace(/#.*$/, ""), sourceLabel(it)];
  if (it.source_title) return [it.source_title, it.source_title];
  if (it.process) return [it.process, it.process.replace(/\.exe$/i, "")];
  return ["", "Other"];
}

/** Shelf-packs items into clusters, and clusters into rows. Returns positions and cluster labels. */
function arrange(list: Item[], origin: [number, number] = [0, 0]) {
  const pos = new Map<string, [number, number]>();
  const lbls: CanvasDoc["labels"] = [];
  const clusters = new Map<string, { label: string; its: Item[] }>();
  for (const it of list) { const [k, label] = clusterOf(it); (clusters.get(k) ?? clusters.set(k, { label, its: [] }).get(k)!).its.push(it); }
  const G = 16, CG = 70, LABEL = 0; // clusters by source page, without a text label
  const boxes = [...clusters.values()].map((c) => {
    const area = c.its.reduce((a, it) => { const [w, h] = cv.size.get(it.id)!; return a + (w + G) * (h + G); }, 0);
    const maxW = Math.min(1100, Math.max(260, Math.sqrt(area) * 1.25));
    let x = 0, y = 0, rowH = 0, bw = 0;
    const local: [Item, number, number][] = [];
    for (const it of c.its) {
      const [w, h] = cv.size.get(it.id)!;
      if (x > 0 && x + w > maxW) { x = 0; y += rowH + G; rowH = 0; }
      local.push([it, x, y]);
      x += w + G;
      rowH = Math.max(rowH, h);
      bw = Math.max(bw, x - G);
    }
    return { label: c.label, local, w: bw, h: y + rowH + LABEL };
  });
  const total = boxes.reduce((a, b) => a + (b.w + CG) * (b.h + CG), 0);
  const worldW = Math.max(1400, Math.sqrt(total) * 1.5);
  let x = 0, y = 0, rowH = 0;
  for (const b of boxes) {
    if (x > 0 && x + b.w > worldW) { x = 0; y += rowH + CG; rowH = 0; }
    for (const [it, lx, ly] of b.local) pos.set(it.id, [origin[0] + x + lx, origin[1] + y + LABEL + ly]);
    x += b.w + CG;
    rowH = Math.max(rowH, b.h);
  }
  return { pos, labels: lbls };
}

function cvBounds(ids: Iterable<string>) {
  const doc = cv.doc!;
  let [l, t, r, b] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const id of ids) {
    const p = doc.pos[id], s = cv.size.get(id);
    if (!p || !s) continue;
    l = Math.min(l, p[0]); t = Math.min(t, p[1]); r = Math.max(r, p[0] + s[0]); b = Math.max(b, p[1] + s[1]);
  }
  return l === Infinity ? { l: 0, t: 0, r: 0, b: 0 } : { l, t, r, b };
}

function cvFit(animate = true) {
  const bb = cvBounds(cv.list.map((i) => i.id));
  const W = canvasEl.clientWidth, H = canvasEl.clientHeight - 60;
  if (!W || bb.r <= bb.l) return;
  const z = Math.min(1.25, Math.max(0.08, Math.min((W - 80) / (bb.r - bb.l), (H - 100) / (bb.b - bb.t))));
  cv.zoom = z;
  cv.pan = [(W - (bb.r - bb.l) * z) / 2 - bb.l * z, 60 + (H - (bb.b - bb.t) * z) / 2 - bb.t * z];
  if (animate && !reduced()) world.animate([{ transform: world.style.transform || "none" }, { transform: `translate(${cv.pan[0]}px, ${cv.pan[1]}px) scale(${z})` }], { duration: 260, easing: EASE });
  cvApply();
  cvSave();
}

function buildCanvas(keepView: boolean) {
  const doc = cv.doc!;
  cv.built = doc.id;
  cv.list = doc.ids.map((id) => items.find((i) => i.id === id)).filter((i): i is Item => !!i).slice(0, CANVAS_MAX);
  const missingN = doc.ids.length - cv.list.length;
  cvNote.textContent = missingN > 0 ? `${missingN} item${missingN === 1 ? " is" : "s are"} in the trash and not shown` : "";
  cvNote.classList.toggle("on", missingN > 0);
  count.textContent = `${cv.list.length} item${cv.list.length === 1 ? "" : "s"} on this canvas`;

  world.replaceChildren();
  const els: HTMLElement[] = [];
  for (const it of cv.list) {
    const d = cvItem(it);
    let w = 240;
    if ((isImg(it) || isVideo(it)) && it.w && it.h) {
      const ar = it.w / it.h;
      w = Math.round(Math.min(380, Math.max(140, Math.sqrt(52000 * ar))));
      d.style.height = `${Math.round(w / ar)}px`;
    } else if (isImg(it)) d.style.height = "180px";
    d.style.width = `${w}px`;
    world.append(d);
    els.push(d);
  }
  els.forEach((d, i) => cv.size.set(cv.list[i].id, [d.offsetWidth, d.offsetHeight]));

  // first open arranges everything; items added later go into free space below
  const missing = cv.list.filter((it) => !doc.pos[it.id]);
  if (missing.length) {
    const placed = cv.list.filter((it) => doc.pos[it.id]).map((i) => i.id);
    const bb = placed.length ? cvBounds(placed) : null;
    const r = arrange(missing, bb ? [bb.l, bb.b + 90] : [0, 0]);
    r.pos.forEach((p, id) => (doc.pos[id] = p));
    doc.labels.push(...r.labels);
    cvSave();
  }
  els.forEach((d, i) => { const p = doc.pos[cv.list[i].id]; d.style.transform = `translate(${p[0]}px, ${p[1]}px)`; });
  if (!keepView) {
    if (doc.pan) { cv.pan = doc.pan; cv.zoom = doc.zoom || 1; cvApply(); } else cvFit(false);
    if (!reduced()) world.animate([{ opacity: 0, transform: `${world.style.transform} scale(.98)` }, { opacity: 1, transform: world.style.transform }], { duration: 260, easing: EASE });
  } else cvApply();
}

function rearrange() {
  const doc = cv.doc!;
  cv.size.clear();
  world.querySelectorAll<HTMLElement>(".cv-item").forEach((d) => cv.size.set(d.dataset.id!, [d.offsetWidth, d.offsetHeight]));
  const r = arrange(cv.list);
  doc.pos = Object.fromEntries(r.pos);
  doc.labels = r.labels;
  for (const d of world.querySelectorAll<HTMLElement>(".cv-item")) {
    const p = doc.pos[d.dataset.id!];
    const to = `translate(${p[0]}px, ${p[1]}px)`;
    if (!reduced()) d.animate([{ transform: d.style.transform }, { transform: to }], { duration: 320, easing: EASE });
    d.style.transform = to;
  }
  cvFit();
}

async function removeFromCanvas(ids: string[]) {
  const doc = cv.doc;
  if (!doc) return;
  selected.clear();
  markSel();
  await burnAll(ids.map((id) => world.querySelector<HTMLElement>(`.cv-item[data-id="${CSS.escape(id)}"]`)).filter((e): e is HTMLElement => !!e));
  await burnsSettled();
  if (cv.doc !== doc) return;
  const before = { ids: [...doc.ids], pos: { ...doc.pos } };
  doc.ids = doc.ids.filter((id) => !ids.includes(id));
  for (const id of ids) delete doc.pos[id];
  selected.clear();
  updateSelBar();
  showDetail();
  buildCanvas(true);
  cvSave();
  flash(`Removed ${ids.length} from this canvas (they stay in your library)`, { label: "Undo", fn: () => { if (cv.doc !== doc) return; doc.ids = before.ids; doc.pos = before.pos; buildCanvas(true); cvSave(); } });
}

// canvas header + toolbar
{
  const b = (host: HTMLElement, label: string, title: string, fn: () => void, cls = "btn") => { const x = el("button", cls, label); x.title = title; x.onclick = fn; host.append(x); return x; };
  b(cvHead, "‹ Canvases", "Back to all canvases (this one is saved)", () => closeCanvasDoc(), "btn ghost");
  cvName.spellcheck = false;
  cvName.title = "Canvas name";
  cvName.addEventListener("input", () => { if (cv.doc) { cv.doc.name = cvName.value.trim() || "Untitled canvas"; cvSave(); } });
  cvName.addEventListener("keydown", (e) => { e.stopPropagation(); if (e.key === "Enter" || e.key === "Escape") cvName.blur(); });
  cvName.addEventListener("change", () => renderRail());
  cvHead.append(cvName, cvStatus, el("span", "grow"));
  const add = b(cvHead, "", "Add items from your library to this canvas", () => openCanvasAdd(), "btn");
  add.innerHTML = ICON.plus; // static markup
  add.append("Add");
  b(cvHead, "Fit", "Show everything", () => cvFit());
  b(cvHead, "Re-arrange", "Arrange again by source page (forgets moved positions)", rearrange);
  b(cvHead, "Save as image", "Save this canvas as a PNG in your library", exportCanvas);
  const zoomBy = (k: number) => {
    const W = canvasEl.clientWidth / 2, H = canvasEl.clientHeight / 2;
    const z = Math.min(3, Math.max(0.08, cv.zoom * k));
    cv.pan = [W - (W - cv.pan[0]) * (z / cv.zoom), H - (H - cv.pan[1]) * (z / cv.zoom)];
    cv.zoom = z;
    cvApply();
    cvSave();
  };
  b(cvBar, "−", "Zoom out", () => zoomBy(1 / 1.25));
  cvBar.append(el("span", "z", "100%"));
  b(cvBar, "+", "Zoom in", () => zoomBy(1.25));
}

// pan / select / drag items
canvasEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  if ((e.target as HTMLElement).closest("#cv-bar, #cv-head, #cvadd")) return;
  const doc = cv.doc;
  if (!doc) return;
  const item = (e.target as HTMLElement).closest<HTMLElement>(".cv-item");
  const [x0, y0] = [e.clientX, e.clientY];
  let moved = false;
  canvasEl.setPointerCapture(e.pointerId);
  if (item && e.button === 0) {
    const id = item.dataset.id!;
    if (!selected.has(id) || e.ctrlKey || e.shiftKey) clickSelect(id, e, cv.list);
    const group = [...selected].filter((s) => doc.pos[s]);
    const start = new Map(group.map((g) => [g, [...doc.pos[g]] as [number, number]]));
    const nodes = new Map(group.map((g) => [g, world.querySelector<HTMLElement>(`.cv-item[data-id="${CSS.escape(g)}"]`)]));
    const move = (m: PointerEvent) => {
      const dx = (m.clientX - x0) / cv.zoom, dy = (m.clientY - y0) / cv.zoom;
      if (!moved && Math.hypot(m.clientX - x0, m.clientY - y0) < 4) return;
      moved = true;
      for (const g of group) {
        const s = start.get(g)!, n = nodes.get(g);
        const p: [number, number] = [Math.round(s[0] + dx), Math.round(s[1] + dy)];
        doc.pos[g] = p;
        if (n) { n.classList.add("drag"); n.style.transform = `translate(${p[0]}px, ${p[1]}px)`; }
      }
    };
    const up = () => {
      canvasEl.removeEventListener("pointermove", move);
      nodes.forEach((n) => n?.classList.remove("drag"));
      if (moved) cvSave();
    };
    canvasEl.addEventListener("pointermove", move);
    canvasEl.addEventListener("pointerup", up, { once: true });
    return;
  }
  const p0 = [...cv.pan];
  canvasEl.classList.add("pan");
  const move = (m: PointerEvent) => {
    if (!moved && Math.hypot(m.clientX - x0, m.clientY - y0) < 3) return;
    moved = true;
    cv.pan = [p0[0] + m.clientX - x0, p0[1] + m.clientY - y0];
    cvApply();
  };
  const up = () => {
    canvasEl.removeEventListener("pointermove", move);
    canvasEl.classList.remove("pan");
    if (moved) cvSave(); else if (selected.size) { selected.clear(); refreshSel(); }
  };
  canvasEl.addEventListener("pointermove", move);
  canvasEl.addEventListener("pointerup", up, { once: true });
});
canvasEl.addEventListener("dblclick", (e) => {
  const item = (e.target as HTMLElement).closest<HTMLElement>(".cv-item");
  if (item) openViewer(cv.list.findIndex((i) => i.id === item.dataset.id), cv.list);
});
canvasEl.addEventListener("wheel", (e) => {
  if ((e.target as HTMLElement).closest("#cv-head, #cvadd")) return;
  e.preventDefault();
  if (e.ctrlKey) {
    const r = canvasEl.getBoundingClientRect();
    const [mx, my] = [e.clientX - r.left, e.clientY - r.top];
    const z = Math.min(3, Math.max(0.08, cv.zoom * Math.exp(-e.deltaY * 0.0015)));
    cv.pan = [mx - (mx - cv.pan[0]) * (z / cv.zoom), my - (my - cv.pan[1]) * (z / cv.zoom)];
    cv.zoom = z;
  } else {
    cv.pan = [cv.pan[0] - (e.shiftKey ? e.deltaY : e.deltaX), cv.pan[1] - (e.shiftKey ? 0 : e.deltaY)];
  }
  cvApply();
  cvSave();
}, { passive: false });

// Add items: a picker over the canvas with every library item not on it yet (search, click to pick, Add).
const cvAdd = el("div");
cvAdd.id = "cvadd";
canvasEl.append(cvAdd);
function openCanvasAdd() {
  const doc = cv.doc;
  if (!doc) return;
  const picked = new Set<string>();
  const head = el("div", "ah");
  const search = el("input", "field");
  search.placeholder = "Search your library…";
  search.spellcheck = false;
  const go = el("button", "btn pri", "Add");
  const cancel = el("button", "btn ghost", "Cancel");
  head.append(el("b", "", "Add to this canvas"), search, cancel, go);
  const list = el("div", "al");
  cvAdd.replaceChildren(head, list);
  const paintGo = () => { go.textContent = picked.size ? `Add ${picked.size}` : "Add"; go.disabled = !picked.size; };
  const render = () => {
    const terms = search.value.toLowerCase().split(/\s+/).filter(Boolean);
    const pool = items.filter((it) => !doc.ids.includes(it.id) && terms.every((t) => haystack(it).includes(t.replace(/^#/, "")))).slice(0, 300);
    list.replaceChildren(...pool.map((it) => {
      const t = el("button", "at" + (picked.has(it.id) ? " on" : ""));
      if (isImg(it)) { const im = el("img"); im.loading = "lazy"; im.draggable = false; im.src = thumb(it); t.append(im); }
      else t.append(el("span", "", (it.text ?? it.caption ?? it.name).slice(0, 140)));
      t.title = it.caption || sourceLabel(it) || it.name;
      t.onclick = () => { picked.has(it.id) ? picked.delete(it.id) : picked.add(it.id); t.classList.toggle("on"); paintGo(); };
      return t;
    }));
    if (!pool.length) list.append(el("div", "ae", "Everything matching is already on this canvas."));
  };
  search.addEventListener("input", render);
  search.addEventListener("keydown", (e) => { e.stopPropagation(); if (e.key === "Escape") close(); if (e.key === "Enter" && picked.size) go.click(); });
  const close = () => cvAdd.classList.remove("on");
  cancel.onclick = close;
  go.onclick = () => {
    if (!picked.size || cv.doc !== doc) return;
    doc.ids.push(...picked);
    close();
    buildCanvas(true); // new items go into free space below the others
    cvSave();
    cvFit();
    flash(`Added ${picked.size} to “${doc.name}”`);
  };
  render();
  paintGo();
  cvAdd.classList.add("on");
  search.focus();
}

/** Renders the canvas to a PNG (thumbnails, notes, labels) and saves it into the library. */
async function exportCanvas() {
  const doc = cv.doc;
  if (!doc) return;
  const ids = cv.list.map((i) => i.id);
  const bb = cvBounds(ids);
  const m = 40;
  const W = bb.r - bb.l + 2 * m, H = bb.b - bb.t + 2 * m;
  const s = Math.min(2, 8000 / Math.max(W, H));
  const c = document.createElement("canvas");
  c.width = Math.round(W * s); c.height = Math.round(H * s);
  const g = c.getContext("2d")!;
  g.scale(s, s);
  g.translate(m - bb.l, m - bb.t);
  g.fillStyle = "#15161c";
  g.fillRect(bb.l - m, bb.t - m, W, H);
  const font = getComputedStyle(document.body).fontFamily;
  const rr = (x: number, y: number, w: number, h: number, r: number) => { g.beginPath(); g.roundRect(x, y, w, h, r); };
  const loadImg = (src: string) => new Promise<HTMLImageElement | null>((res) => { const im = new Image(); im.crossOrigin = "anonymous"; im.onload = () => res(im); im.onerror = () => res(null); im.src = src; });
  for (const it of cv.list) {
    const p = doc.pos[it.id], sz = cv.size.get(it.id);
    if (!p || !sz) continue;
    const [x, y] = p, [w, h] = sz;
    g.save();
    rr(x, y, w, h, 12);
    g.clip();
    if (isImg(it)) {
      const im = await loadImg(thumb(it));
      if (im) g.drawImage(im, x, y, w, h); else { g.fillStyle = "#2a2b33"; g.fill(); }
    } else {
      g.fillStyle = isNote(it) ? "#2c2d38" : "#24252e";
      g.fillRect(x, y, w, h);
      g.fillStyle = "#f2f2f5";
      g.font = `13px ${font}`;
      const text = (it.text ?? it.caption ?? it.name).replace(/\r/g, "");
      let ty = y + 26;
      for (const para of text.split("\n")) {
        let line = "";
        for (const word of para.split(/\s+/)) {
          const t = line ? line + " " + word : word;
          if (g.measureText(t).width > w - 28 && line) { g.fillText(line, x + 14, ty); ty += 19; line = word; } else line = t;
          if (ty > y + h - 30) break;
        }
        if (ty > y + h - 30) break;
        g.fillText(line, x + 14, ty);
        ty += 19;
      }
      const src = sourceLabel(it);
      if (src) { g.fillStyle = "rgba(255,255,255,.5)"; g.font = `11px ${font}`; g.fillText(src.slice(0, 40), x + 14, y + h - 12); }
    }
    g.restore();
    g.strokeStyle = "rgba(255,255,255,.12)";
    rr(x + 0.5, y + 0.5, w - 1, h - 1, 12);
    g.stroke();
  }
  try {
    const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/png"));
    if (!blob) throw new Error("could not render the canvas");
    const name = `${doc.name.replace(/[\\/:*?"<>|]+/g, "-").slice(0, 60) || "canvas"}.png`;
    await invoke("gallery_import_data", new Uint8Array(await blob.arrayBuffer()), { headers: { "x-name": encodeURIComponent(name) } });
    flash("Saved the canvas as an image in your library");
  } catch (e) {
    flash(String(e));
  }
}

// ---------- background palette + OCR indexing ----------
let backfilling = false;
async function backfill() {
  if (backfilling) return;
  backfilling = true;
  try {
    for (;;) {
      const r = await invoke<{ done: { id: string; palette: string[]; ocr: string | null }[]; remaining: number }>("gallery_backfill", { limit: 6 });
      for (const d of r.done) {
        const it = items.find((i) => i.id === d.id);
        if (it) { if (d.palette.length) it.palette = d.palette; if (d.ocr) it.ocr = d.ocr; }
      }
      if (r.done.length) renderRail();
      if (!r.remaining || !r.done.length) break;
    }
  } catch { /* window closed or indexing unavailable */ }
  backfilling = false;
}

// ---------- actions ----------
function flash(msg: string, action?: { label: string; fn: () => void }) {
  toast.replaceChildren(el("span", "", msg));
  if (action) { const a = el("a", "", action.label); a.onclick = () => { action.fn(); toast.classList.remove("on"); }; toast.append(a); }
  toast.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => toast.classList.remove("on"), action ? 7000 : 2200);
}

const copyItem = (it: Item) => invoke("gallery_copy", { id: it.id }).then(() => flash("Copied")).catch((e) => flash(String(e)));

/** Trash: cards shrink out and neighbours glide in; Undo (or the Trash tab) brings them back. */
async function trash(ids: string[]) {
  const kept = ids.filter((id) => byId(id)?.favorite);
  ids = ids.filter((id) => !byId(id)?.favorite);
  if (kept.length) flash(ids.length ? `Kept ${kept.length} favourite${kept.length === 1 ? "" : "s"} (un-star to delete)` : "Favourites can't be deleted: un-star first");
  if (!ids.length) return;
  const burning = burnCards(ids);
  selected.clear();
  markSel();
  showDetail();
  let moved: string[];
  try {
    moved = await invoke<string[]>("gallery_delete", { ids });
  } catch (e) {
    await burning;
    grid.querySelectorAll<HTMLElement>(".card.burning").forEach(unburn);
    flash(String(e));
    return;
  }
  await burning;
  await burnsSettled(); // deleting again while something burns: re-arrange once, after the last one
  selected.clear();
  items = items.filter((i) => !moved.includes(i.id));
  renderRail();
  filter("flip");
  updateSelBar();
  showDetail();
  flash(`Moved ${moved.length} to the trash`, { label: "Undo", fn: async () => { await invoke("gallery_restore", { ids: moved }); await load(true); } });
}
async function restore(ids: string[]) {
  await invoke("gallery_restore", { ids });
  selected.clear();
  trashItems = trashItems.filter((i) => !ids.includes(i.id));
  filter("flip");
  updateSelBar();
  showDetail();
  renderRail();
  flash(`Restored ${ids.length} item${ids.length === 1 ? "" : "s"}`);
  load(false);
}
async function purge(ids: string[]) {
  const burning = burnCards(ids);
  selected.clear();
  markSel();
  showDetail();
  try {
    await invoke("gallery_purge", { ids });
  } catch (e) {
    await burning;
    grid.querySelectorAll<HTMLElement>(".card.burning").forEach(unburn);
    flash(String(e));
    return;
  }
  await burning;
  await burnsSettled();
  selected.clear();
  trashItems = trashItems.filter((i) => !ids.includes(i.id));
  filter("flip");
  updateSelBar();
  showDetail();
  renderRail();
  flash(`Deleted ${ids.length} item${ids.length === 1 ? "" : "s"} for good`);
}

// ---------- View menu: card size, spacing and what cards show ----------
const viewBtn = $("viewBtn"), viewPop = $("viewpop");
function applyView() {
  document.body.classList.toggle("nocap", !view.captions);
  document.body.classList.toggle("nosrc", !view.sources);
  grid.classList.toggle("square", view.square);
  store.set("stash.gallery.view", JSON.stringify(view));
}
let viewRelayout: number | undefined;
function setView(patch: Partial<ViewPrefs>, now = false) {
  Object.assign(view, patch);
  applyView();
  clearTimeout(viewRelayout);
  viewRelayout = window.setTimeout(() => { if (itemTab() || tab === "trash") { layout("flip"); renderScrub(); } }, now ? 0 : 60);
}
/** Glass slider (same look as Settings). */
function viewSlider(min: number, max: number, step: number, get: () => number, set: (v: number) => void, fmt: (v: number) => string) {
  const wrapEl = el("div", "vrow");
  const s = el("div", "slider");
  const track = el("div", "track"), fill = el("div", "fill"), knob = el("div", "knob");
  track.append(fill);
  s.append(track, knob);
  s.tabIndex = 0;
  const val = el("span", "vv");
  const paint = () => {
    const fr = (get() - min) / (max - min);
    knob.style.left = `calc(13px + (100% - 26px) * ${fr})`;
    fill.style.width = `calc(13px + (100% - 26px) * ${fr})`;
    val.textContent = fmt(get());
  };
  const at = (x: number) => { const r = s.getBoundingClientRect(); const fr = Math.min(1, Math.max(0, (x - r.left - 13) / (r.width - 26))); return Math.round((min + fr * (max - min)) / step) * step; };
  s.addEventListener("pointerdown", (e) => {
    s.setPointerCapture(e.pointerId); s.classList.add("drag");
    const mv = (m: PointerEvent) => { set(at(m.clientX)); paint(); };
    mv(e);
    s.addEventListener("pointermove", mv);
    s.addEventListener("pointerup", () => { s.classList.remove("drag"); s.removeEventListener("pointermove", mv); }, { once: true });
  });
  s.addEventListener("keydown", (e) => {
    const d = e.key === "ArrowRight" || e.key === "ArrowUp" ? step : e.key === "ArrowLeft" || e.key === "ArrowDown" ? -step : 0;
    if (d) { e.preventDefault(); e.stopPropagation(); set(Math.min(max, Math.max(min, get() + d))); paint(); }
  });
  paint();
  wrapEl.append(s, val);
  return { el: wrapEl, paint };
}
function buildViewPop() {
  viewPop.replaceChildren(el("h4", "", "View"));
  const size = viewSlider(140, 480, 10, () => view.size, (v) => setView({ size: v }), (v) => (v <= 170 ? "Small" : v <= 260 ? "Medium" : v <= 360 ? "Large" : "Huge"));
  const gap = viewSlider(4, 28, 2, () => view.gap, (v) => setView({ gap: v }), (v) => `${v} px`);
  const tog = (label: string, key: "captions" | "days" | "sources" | "square") => {
    const r = el("label", "vt");
    const sw = el("label", "switch");
    const i = el("input");
    i.type = "checkbox";
    i.checked = view[key];
    i.addEventListener("change", () => setView({ [key]: i.checked }, true));
    sw.append(i, el("i"));
    r.append(el("span", "", label), sw);
    return r;
  };
  const reset = el("button", "btn ghost", "Reset");
  reset.onclick = () => { setView({ ...VIEW_DEFAULT }, true); buildViewPop(); };
  viewPop.append(el("div", "vl", "Card size"), size.el, el("div", "vl", "Spacing"), gap.el,
    tog("Square tiles", "square"), tog("Captions", "captions"), tog("Note sources", "sources"), tog("Day labels", "days"), reset);
  viewSizePaint = size.paint;
}
let viewSizePaint = () => {};
viewBtn.addEventListener("click", (e) => {
  e.stopPropagation();
  if (viewPop.classList.toggle("on")) buildViewPop();
});
document.addEventListener("pointerdown", (e) => { if (!(e.target as HTMLElement).closest("#viewpop, #viewBtn")) viewPop.classList.remove("on"); });
// Ctrl + mouse wheel over the grid resizes the cards
wrap.addEventListener("wheel", (e) => {
  if (!e.ctrlKey) return;
  e.preventDefault();
  setView({ size: Math.min(480, Math.max(140, view.size + (e.deltaY < 0 ? 20 : -20))) });
  viewSizePaint();
}, { passive: false });
applyView();

// ---------- favourites ----------
async function setFav(ids: string[], on: boolean) {
  const list = ids.map((id) => byId(id)).filter((i): i is Item => !!i && !i.trashed);
  if (!list.length) return;
  try {
    await invoke("gallery_favorite", { ids: list.map((i) => i.id), on });
  } catch (e) { flash(String(e)); return; }
  for (const it of list) {
    it.favorite = on;
    refreshItem(it);
    const fb = grid.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(it.id)}"]:not(.leaving) .fb`);
    if (fb && on && !reduced()) { fb.classList.remove("pop"); void fb.offsetWidth; fb.classList.add("pop"); }
  }
  updateSelBar();
  if (selected.size) showDetail();
  if (tab === "favs") filter("flip");
  flash(on ? (list.length === 1 ? "Added to Favourites" : `Added ${list.length} to Favourites`) : list.length === 1 ? "Removed from Favourites" : `Removed ${list.length} from Favourites`);
}
function toggleFavSel() {
  const sel = [...selected].map((id) => byId(id)).filter((i): i is Item => !!i);
  if (sel.length && tab !== "trash") setFav(sel.map((s) => s.id), !sel.every((s) => s.favorite));
}
// The holographic sheen on favourites leans toward the pointer.
grid.addEventListener("pointermove", (e) => {
  const c = (e.target as HTMLElement).closest<HTMLElement>(".card.fav");
  if (!c) return;
  const r = c.getBoundingClientRect();
  c.style.setProperty("--hx", `${(((e.clientX - r.left) / r.width) * 100).toFixed(1)}%`);
  c.style.setProperty("--hy", `${(((e.clientY - r.top) / r.height) * 100).toFixed(1)}%`);
}, { passive: true });

// ---------- burn: deleting items ----------
// A deleted card burns away. Every burn is different: it catches at a random corner, a point on any edge, or two
// points at once, and the front's raggedness varies. The unburnt part is a clip-path traced from a noise field
// (marching squares, smoothed with quadratic curves, so the edge is clean at any size); the charred band and the
// glowing rim are vector strokes along the same curve on a full-resolution canvas; embers and a little smoke rise.
const BURN_MAX = 60; // everything on screen burns together; beyond this, cards simply shrink out

/** Multi-octave value noise on an nx x ny node grid, 0..1. `scale` stretches the blobs. */
function valueNoise(nx: number, ny: number, scale: number): Float32Array {
  const out = new Float32Array(nx * ny);
  let amp = 1, total = 0;
  for (const base of [11, 5.5, 2.6]) {
    const cell = base * scale;
    const gx = Math.ceil(nx / cell) + 2, gy = Math.ceil(ny / cell) + 2;
    const g = Float32Array.from({ length: gx * gy }, Math.random);
    for (let y = 0; y < ny; y++) for (let x = 0; x < nx; x++) {
      const fx = x / cell, fy = y / cell, ix = Math.floor(fx), iy = Math.floor(fy);
      const sx = (fx - ix) * (fx - ix) * (3 - 2 * (fx - ix)), sy = (fy - iy) * (fy - iy) * (3 - 2 * (fy - iy));
      const a = g[iy * gx + ix], b = g[iy * gx + ix + 1], c = g[(iy + 1) * gx + ix], d = g[(iy + 1) * gx + ix + 1];
      out[y * nx + x] += (a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy) * amp;
    }
    total += amp;
    amp *= 0.5;
  }
  for (let i = 0; i < out.length; i++) out[i] /= total;
  return out;
}

/** Closed loops (node coordinates) around the region where field >= t (marching squares, saddles by centre). */
function contour(f: Float32Array, nx: number, ny: number, t: number): [number, number][][] {
  const pts = new Map<number, [number, number]>();
  const nbr = new Map<number, number[]>();
  const edge = (x: number, y: number, vert: boolean) => {
    const id = (y * nx + x) * 2 + (vert ? 1 : 0);
    if (!pts.has(id)) {
      const a = f[y * nx + x], b = vert ? f[(y + 1) * nx + x] : f[y * nx + x + 1];
      const k = Math.min(1, Math.max(0, (t - a) / (b - a || 1e-6)));
      pts.set(id, vert ? [x, y + k] : [x + k, y]);
    }
    return id;
  };
  const link = (a: number, b: number) => { (nbr.get(a) ?? nbr.set(a, []).get(a)!).push(b); (nbr.get(b) ?? nbr.set(b, []).get(b)!).push(a); };
  for (let y = 0; y < ny - 1; y++) for (let x = 0; x < nx - 1; x++) {
    const tl = f[y * nx + x] >= t, tr = f[y * nx + x + 1] >= t, br = f[(y + 1) * nx + x + 1] >= t, bl = f[(y + 1) * nx + x] >= t;
    const c = (tl ? 8 : 0) | (tr ? 4 : 0) | (br ? 2 : 0) | (bl ? 1 : 0);
    if (c === 0 || c === 15) continue;
    const T = () => edge(x, y, false), B = () => edge(x, y + 1, false), L = () => edge(x, y, true), R = () => edge(x + 1, y, true);
    switch (c) {
      case 1: case 14: link(L(), B()); break;
      case 2: case 13: link(B(), R()); break;
      case 3: case 12: link(L(), R()); break;
      case 4: case 11: link(T(), R()); break;
      case 6: case 9: link(T(), B()); break;
      case 7: case 8: link(L(), T()); break;
      case 5: case 10: {
        const mid = (f[y * nx + x] + f[y * nx + x + 1] + f[(y + 1) * nx + x + 1] + f[(y + 1) * nx + x]) / 4 >= t;
        if ((c === 5) === mid) { link(L(), T()); link(B(), R()); } else { link(L(), B()); link(T(), R()); }
      }
    }
  }
  const loops: [number, number][][] = [];
  const seen = new Set<number>();
  for (const start of nbr.keys()) {
    if (seen.has(start)) continue;
    const loop: [number, number][] = [];
    let prev = -1, cur = start;
    while (!seen.has(cur)) {
      seen.add(cur);
      loop.push(pts.get(cur)!);
      const next = (nbr.get(cur) ?? []).find((n) => n !== prev && !seen.has(n));
      if (next === undefined) break;
      prev = cur;
      cur = next;
    }
    if (loop.length > 2) loops.push(loop);
  }
  return loops;
}

/** Smooth closed path through the points (quadratic curves between segment midpoints). */
function smoothClosed(p: [number, number][]): string {
  const n = p.length, f = (v: number) => v.toFixed(1);
  const mid = (a: [number, number], b: [number, number]) => `${f((a[0] + b[0]) / 2)} ${f((a[1] + b[1]) / 2)}`;
  let d = `M${mid(p[n - 1], p[0])}`;
  for (let i = 0; i < n; i++) d += `Q${f(p[i][0])} ${f(p[i][1])} ${mid(p[i], p[(i + 1) % n])}`;
  return d + "Z";
}
/** Smooth open path (for the strokes, which skip the parts of a loop that run outside the card). */
function smoothOpen(p: [number, number][], path: Path2D) {
  if (p.length < 2) return;
  path.moveTo(p[0][0], p[0][1]);
  for (let i = 1; i < p.length - 1; i++) path.quadraticCurveTo(p[i][0], p[i][1], (p[i][0] + p[i + 1][0]) / 2, (p[i][1] + p[i + 1][1]) / 2);
  path.lineTo(p[p.length - 1][0], p[p.length - 1][1]);
}

interface Ember { x: number; y: number; vx: number; vy: number; life: number; age: number; size: number; smoke: boolean }

/** Burns currently running; re-arranging after a delete waits until every one of them is done. */
let burnsActive = 0;
let burnWaiters: (() => void)[] = [];
const burnsSettled = () => (burnsActive ? new Promise<void>((r) => burnWaiters.push(r)) : Promise.resolve());

/** Burns an element away (gallery card, canvas item, canvas tile); resolves once it is gone (embers keep rising
 *  for a moment after). `cell`: noise grid size in px (coarser when many burn at once). */
function burn(card: HTMLElement, cell = 5): Promise<void> {
  if (reduced() || !card.isConnected) return Promise.resolve();
  const p = card.parentElement === grid ? posOf.get(card) : undefined;
  const W = card.offsetWidth, H = card.offsetHeight;
  if (!W || !H) return Promise.resolve();
  const box = card.getBoundingClientRect();
  const zk = box.width / W; // on-screen scale (canvas zoom)
  burnsActive++;
  // nodes 1..gw+1 cover the card (node i at x = (i - 1) * cw); the outer ring is always burnt
  const gw = Math.max(12, Math.min(90, Math.round(W / cell))), gh = Math.max(6, Math.min(150, Math.round(H / cell)));
  const cw = W / gw, chh = H / gh;
  const nx = gw + 3, ny = gh + 3;
  const noise = valueNoise(nx, ny, 0.7 + Math.random() * 0.7);
  // where it catches: a corner, a point on an edge, or two points
  const onEdge = (): [number, number] => {
    const s = Math.floor(Math.random() * 4), k = 0.1 + Math.random() * 0.8;
    return s === 0 ? [k * W, -4] : s === 1 ? [W + 4, k * H] : s === 2 ? [k * W, H + 4] : [-4, k * H];
  };
  const corner = (): [number, number] => [Math.random() < 0.5 ? -4 : W + 4, Math.random() < 0.5 ? -4 : H + 4];
  const mode = Math.random();
  const sparks: [number, number][] = mode < 0.35 ? [corner()] : mode < 0.75 ? [onEdge()] : [onEdge(), onEdge()];
  const ragged = 0.28 + Math.random() * 0.16;
  const f = new Float32Array(nx * ny);
  let lo = Infinity, hi = -Infinity;
  for (let y = 0; y < ny; y++) for (let x = 0; x < nx; x++) {
    const i = y * nx + x;
    if (x === 0 || y === 0 || x === nx - 1 || y === ny - 1) { f[i] = -1; continue; }
    const px = (x - 1) * cw, py = (y - 1) * chh;
    const d = Math.min(...sparks.map(([sx, sy]) => Math.hypot(px - sx, py - sy)));
    f[i] = d;
    lo = Math.min(lo, d); hi = Math.max(hi, d);
  }
  for (let i = 0; i < f.length; i++) if (f[i] >= 0) f[i] = (1 - ragged) * ((f[i] - lo) / (hi - lo || 1)) + ragged * noise[i];
  lo = Infinity; hi = -Infinity;
  for (let i = 0; i < f.length; i++) if (f[i] >= 0) { lo = Math.min(lo, f[i]); hi = Math.max(hi, f[i]); }
  for (let i = 0; i < f.length; i++) if (f[i] >= 0) f[i] = (f[i] - lo) / (hi - lo || 1);
  const ms = 950 + Math.min(450, Math.sqrt(W * H) * 0.9);

  card.classList.add("burning");
  const dpr = Math.min(2, devicePixelRatio || 1);
  // char + glowing rim: strokes along the front on a canvas the size of the card (clipped with it)
  const fxc = el("canvas", "burnfx");
  fxc.width = Math.round(W * dpr); fxc.height = Math.round(H * dpr);
  Object.assign(fxc.style, { left: "0", top: "0", width: `${W}px`, height: `${H}px` });
  card.append(fxc);
  const fctx = fxc.getContext("2d")!;
  fctx.lineCap = "round"; fctx.lineJoin = "round";
  // embers and smoke, in the grid around the card (they rise past its top)
  // gallery cards: in the grid (scrolls with it); anything else: a fixed layer over the window
  const PAD = 40, TOP = 130, EW = W * zk + PAD * 2, EH = H * zk + TOP + PAD;
  const ec = el("canvas", "embers");
  ec.width = Math.round(EW * dpr); ec.height = Math.round(EH * dpr);
  Object.assign(ec.style, { width: `${EW}px`, height: `${EH}px` });
  if (p) { ec.style.transform = `translate(${p.x - PAD}px, ${p.y - TOP}px)`; grid.append(ec); }
  else { ec.classList.add("fixed"); Object.assign(ec.style, { left: `${box.left - PAD}px`, top: `${box.top - TOP}px` }); document.body.append(ec); }
  const ectx = ec.getContext("2d")!;
  ectx.scale(dpr, dpr);
  const parts: Ember[] = [];
  const inside = (x: number, y: number) => x > 0.5 && y > 0.5 && x < W - 0.5 && y < H - 0.5;

  return new Promise<void>((resolve) => {
    let t0 = 0, last = 0, done = false;
    const frame = (now: number) => {
      if (!t0) { t0 = now; last = now; }
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      const u = Math.min(1, (now - t0) / ms);
      const t = -0.03 + 1.08 * u ** 1.4;
      ectx.clearRect(0, 0, EW, EH);
      if (!done) {
        const loops = u < 1 ? contour(f, nx, ny, t).map((l) => l.map(([x, y]) => [(x - 1) * cw, (y - 1) * chh] as [number, number])) : [];
        card.style.clipPath = loops.length ? `path(evenodd, "${loops.map(smoothClosed).join("")}")` : "inset(50%)";
        // the front: the parts of the outline that lie inside the card
        const front = new Path2D();
        const pts: [number, number][] = [];
        for (const l of loops) {
          let run: [number, number][] = [];
          for (const q of [...l, l[0]]) {
            if (inside(q[0], q[1])) { run.push(q); pts.push(q); } else { smoothOpen(run, front); run = []; }
          }
          smoothOpen(run, front);
        }
        fctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        fctx.clearRect(0, 0, W, H);
        fctx.globalCompositeOperation = "source-over";
        for (const [w, a] of [[30, 0.1], [20, 0.16], [12, 0.24], [6, 0.34]] as const) {
          fctx.lineWidth = w;
          fctx.strokeStyle = `rgba(28,12,4,${a})`;
          fctx.stroke(front);
        }
        const flick = 0.85 + Math.random() * 0.15;
        fctx.globalCompositeOperation = "lighter";
        for (const [w, col] of [[9, `rgba(255,90,10,${0.22 * flick})`], [4.5, `rgba(255,140,40,${0.75 * flick})`], [1.6, `rgba(255,236,190,${0.95 * flick})`]] as const) {
          fctx.lineWidth = w;
          fctx.strokeStyle = col;
          fctx.stroke(front);
        }
        // embers (and now and then a puff of smoke) from points on the front
        if (pts.length) for (let k = 0; k < 2; k++) {
          if (Math.random() > 0.4) continue;
          const [x, y] = pts[(Math.random() * pts.length) | 0];
          const smoke = Math.random() < 0.1;
          parts.push({ x: x * zk + PAD, y: y * zk + TOP, vx: (Math.random() - 0.5) * 40, vy: -(smoke ? 25 + Math.random() * 25 : 45 + Math.random() * 95), life: smoke ? 0.8 + Math.random() * 0.6 : 0.35 + Math.random() * 0.5, age: 0, size: smoke ? 4 + Math.random() * 4 : 0.6 + Math.random() * 0.9, smoke });
        }
        // warm light spilling past the edge
        ectx.globalCompositeOperation = "lighter";
        for (let k = 0; k < Math.min(pts.length, 5); k++) {
          const [x, y] = pts[(Math.random() * pts.length) | 0];
          const [gx, gy] = [x * zk + PAD, y * zk + TOP];
          const gr = ectx.createRadialGradient(gx, gy, 0, gx, gy, 18);
          gr.addColorStop(0, "rgba(255,140,40,.08)");
          gr.addColorStop(1, "rgba(255,90,20,0)");
          ectx.fillStyle = gr;
          ectx.fillRect(gx - 18, gy - 18, 36, 36);
        }
        if (u >= 1) {
          done = true;
          card.classList.add("burned");
          fxc.remove();
          resolve();
          if (--burnsActive === 0) { const w = burnWaiters; burnWaiters = []; w.forEach((r) => r()); }
        }
      }
      // embers rise and sway, cool from yellow to red and fade; smoke swells and thins
      for (const e of parts) {
        e.age += dt;
        e.vy -= (e.smoke ? 10 : 40) * dt;
        e.vx += Math.sin((e.age + e.x) * 9) * 60 * dt;
        e.x += e.vx * dt; e.y += e.vy * dt;
      }
      for (let i = parts.length - 1; i >= 0; i--) if (parts[i].age >= parts[i].life) parts.splice(i, 1);
      ectx.globalCompositeOperation = "source-over";
      for (const e of parts) if (e.smoke) {
        const k = e.age / e.life, r = e.size * (1 + k * 2.4);
        const gr = ectx.createRadialGradient(e.x, e.y, 0, e.x, e.y, r);
        gr.addColorStop(0, `rgba(150,150,160,${(0.09 * Math.sin(Math.PI * k)).toFixed(3)})`);
        gr.addColorStop(1, "rgba(150,150,160,0)");
        ectx.fillStyle = gr;
        ectx.fillRect(e.x - r, e.y - r, r * 2, r * 2);
      }
      ectx.globalCompositeOperation = "lighter";
      for (const e of parts) if (!e.smoke) {
        const k = e.age / e.life, a = (1 - k) * (0.75 + Math.random() * 0.25);
        const [r, g, b] = k < 0.35 ? [255, 225, 140] : k < 0.7 ? [255, 140, 40] : [230, 60, 20];
        ectx.fillStyle = `rgba(${r},${g},${b},${(a * 0.16).toFixed(3)})`;
        ectx.beginPath(); ectx.arc(e.x, e.y, e.size * 2.6, 0, Math.PI * 2); ectx.fill();
        ectx.fillStyle = `rgba(${r},${g},${b},${a.toFixed(3)})`;
        ectx.beginPath(); ectx.arc(e.x, e.y, e.size, 0, Math.PI * 2); ectx.fill();
      }
      if (!done || parts.length) requestAnimationFrame(frame);
      else ec.remove();
    };
    requestAnimationFrame(frame);
  });
}

/** Burns elements all at once (a coarser grid when there are many, so it stays smooth). */
function burnAll(els: HTMLElement[]): Promise<unknown> {
  if (reduced()) return Promise.resolve();
  const list = els.slice(0, BURN_MAX);
  const cell = list.length > 24 ? 9 : list.length > 8 ? 7 : 5;
  return Promise.all(list.map((e) => burn(e, cell)));
}
/** Burns the gallery cards among `ids` that are on screen, all together; the rest just leave. */
function burnCards(ids: string[]): Promise<unknown> {
  const wr = wrap.getBoundingClientRect();
  const els: HTMLElement[] = [];
  for (const id of ids) {
    const c = grid.querySelector<HTMLElement>(`.card[data-id="${CSS.escape(id)}"]:not(.leaving)`);
    const r = c?.getBoundingClientRect();
    if (c && r && r.bottom > wr.top && r.top < wr.bottom) els.push(c);
  }
  return burnAll(els);
}
/** Undoes a burn on an element that comes back (Undo, Restore, a failed delete). */
function unburn(c: HTMLElement) {
  if (!c.classList.contains("burning")) return;
  c.classList.remove("burning", "burned");
  c.style.clipPath = "";
  c.querySelector(".burnfx")?.remove();
}

// ---------- events ----------
q.addEventListener("input", () => { if (tab === "canvases") setTab("all"); filter("none"); paintSugg(); }); // typing stays instant

// Search suggestions: the word being typed autofills to a tag (#moodboard), a colour (color:blue) or an app
// (app:chrome). ↑/↓ pick, Enter or Tab takes one, Esc closes.
const qs = $("qsugg");
let qsHi = 0, qsList: { label: string; token: string; n: number; kind: string; swatch?: string; pick: () => void }[] = [];
function paintSugg() {
  const words = q.value.split(/\s+/);
  const word = (words[words.length - 1] ?? "").toLowerCase().replace(/^(#|color:|app:)/, "");
  qsList = [];
  if (word && document.activeElement === q) {
    for (const [t, n] of allTags()) if (t.toLowerCase().includes(word)) qsList.push({ label: "#" + t, token: "#" + t, n, kind: isFavTag(t) ? "★ Tag" : "Tag", pick: () => (f.tag = t) });
    qsList = qsList.slice(0, 6);
    for (const [name, hex] of BUCKETS) if (name.startsWith(word)) qsList.push({ label: name, token: "color:" + name, n: items.filter((i) => hasColor(i, name)).length, kind: "Colour", swatch: hex, pick: () => (f.color = name) });
    const apps = new Map<string, number>();
    for (const it of items) if (it.process) apps.set(it.process, (apps.get(it.process) ?? 0) + 1);
    for (const [a, n] of apps) { const name = a.replace(/\.exe$/i, ""); if (name.toLowerCase().startsWith(word)) qsList.push({ label: name, token: "app:" + name.toLowerCase(), n, kind: "App", pick: () => (f.app = a) }); }
    qsList = qsList.filter((x) => x.n > 0 && x.token.toLowerCase() !== words[words.length - 1].toLowerCase()).slice(0, 9);
  }
  qsHi = Math.min(qsHi, Math.max(0, qsList.length - 1));
  qs.replaceChildren(...qsList.map((x, i) => {
    const b = el("button", i === qsHi ? "hi" : "");
    if (x.swatch) { const d = el("i"); d.style.background = x.swatch; b.append(d); }
    b.append(el("b", "", x.label), el("span", "", `${x.kind} · ${x.n}`));
    b.onmousedown = (e) => { e.preventDefault(); takeSugg(i); };
    return b;
  }));
  qs.classList.toggle("on", qsList.length > 0);
}
function takeSugg(i: number) {
  const x = qsList[i];
  if (!x) return;
  x.pick(); // shows as a filter chip in the header
  q.value = "";
  qsList = [];
  qs.classList.remove("on");
  renderRail();
  filter("switch");
}
q.addEventListener("keydown", (e) => {
  if (!qs.classList.contains("on")) return;
  if (e.key === "ArrowDown") { e.preventDefault(); qsHi = (qsHi + 1) % qsList.length; paintSugg(); }
  else if (e.key === "ArrowUp") { e.preventDefault(); qsHi = (qsHi - 1 + qsList.length) % qsList.length; paintSugg(); }
  else if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); takeSugg(qsHi); }
  else if (e.key === "Escape") { e.stopPropagation(); qs.classList.remove("on"); }
});
q.addEventListener("blur", () => qs.classList.remove("on"));
q.addEventListener("focus", paintSugg);

window.addEventListener("keydown", (e) => {
  const inField = (e.target as HTMLElement).tagName === "INPUT";
  if (viewer.classList.contains("on")) {
    if (e.key === "Escape") closeViewer();
    else if (e.key === "ArrowRight" || e.key === " " || e.key === "Enter") { e.preventDefault(); stepViewer(1); }
    else if (e.key === "ArrowLeft") stepViewer(-1);
    return;
  }
  const canv = tab === "canvases";
  if (e.ctrlKey && e.key.toLowerCase() === "f") { e.preventDefault(); q.focus(); q.select(); }
  else if (e.key === "/" && !inField) { e.preventDefault(); q.focus(); }
  else if (!inField && itemTab() && e.key.toLowerCase() === "f" && !e.ctrlKey && selected.size) { e.preventDefault(); toggleFavSel(); }
  else if (!inField && itemTab() && e.key.toLowerCase() === "t" && !e.ctrlKey && selected.size) { e.preventDefault(); openTagPop(); }
  else if (e.key === "Escape") {
    if (inField) (e.target as HTMLElement).blur();
    else if (pop.classList.contains("on")) pop.classList.remove("on");
    else if (selected.size) { selected.clear(); refreshSel(); }
    else if (canv && cv.doc) closeCanvasDoc();
  }
  else if (!inField && e.key === "Delete" && selected.size) { if (canv) removeFromCanvas([...selected]); else if (tab !== "trash") trash([...selected]); }
  else if (!inField && (e.key === " " || e.key === "Enter") && selected.size === 1) { e.preventDefault(); const id = [...selected][0]; if (canv) openViewer(cv.list.findIndex((i) => i.id === id), cv.list); else openViewer(shown.findIndex((i) => i.id === id)); }
  else if (!inField && e.ctrlKey && e.key.toLowerCase() === "a") { e.preventDefault(); selected = new Set((canv ? cv.list : shown).map((i) => i.id)); refreshSel(); }
  else if (!inField && !canv && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) { e.preventDefault(); moveSel(e.key); }
});

// ---------- add content: drop and paste ----------
// Drops are handled as HTML5 events (the native handler only understands files): files from Explorer or a
// browser, images, text and links. Our own drag-out coming back in is ignored.
const hasPayload = (dt: DataTransfer | null) => !!dt && ["Files", "text/plain", "text/uri-list", "text/html"].some((t) => dt.types.includes(t));
let dragDepth = 0;
window.addEventListener("dragenter", (e) => { if (dragging || !hasPayload(e.dataTransfer)) return; e.preventDefault(); dragDepth++; dropEl.classList.add("on"); });
window.addEventListener("dragover", (e) => { if (dragging || !hasPayload(e.dataTransfer)) return; e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = "copy"; });
window.addEventListener("dragleave", () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) dropEl.classList.remove("on"); });
window.addEventListener("drop", async (e) => {
  e.preventDefault();
  dragDepth = 0;
  dropEl.classList.remove("on");
  const dt = e.dataTransfer;
  if (dragging || !dt) return;
  let n = 0;
  try {
    if (dt.files.length) {
      for (const fl of Array.from(dt.files)) {
        await invoke("gallery_import_data", new Uint8Array(await fl.arrayBuffer()), { headers: { "x-name": encodeURIComponent(fl.name || "dropped") } });
        n++;
      }
    } else {
      const text = (dt.getData("text/uri-list") || dt.getData("text/plain")).split(/\r?\n/).find((l) => l.trim() && !l.startsWith("#"))?.trim() ?? dt.getData("text/plain").trim();
      if (text) { await invoke("gallery_import_text", { text }); n++; }
    }
  } catch (err) {
    flash(String(err));
    return;
  }
  if (n) flash(`Added ${n} item${n === 1 ? "" : "s"}`); // the library-changed event brings them in
});

// Ctrl+V pastes the clipboard (files, an image, text or a link) into the library.
window.addEventListener("keydown", async (e) => {
  if (!(e.ctrlKey && e.key.toLowerCase() === "v") || (e.target as HTMLElement).tagName === "INPUT" || viewer.classList.contains("on")) return;
  e.preventDefault();
  try {
    flash(await invoke<string>("gallery_paste"));
  } catch (err) {
    flash(String(err));
  }
});

// New captures, tags from the tag radial, imports: Rust emits library-changed and they slide into place.
listen("library-changed", () => load(true).then(() => { if (tab === "canvases" && cv.doc) buildCanvas(true); }).catch(() => {}));

// Opening: the window is created hidden and the page stays invisible (body.pre) while the first layout runs;
// the window is then shown and the entrance (body.opening + the card cascade) plays on the first visible frame.
document.body.classList.add("pre");
invoke<string[]>("gallery_fav_tags").then((t) => { favTags = t; renderRail(); }).catch(() => {});
invoke<{ config: { trash_days?: number } }>("settings_get").then((d) => { trashDays = d.config.trash_days ?? 30; renderTrashBar(); }).catch(() => {});
Promise.all([document.fonts?.ready.catch(() => {}), loadCanvases(), tab === "trash" ? invoke<Item[]>("gallery_trash_list").then((t) => { trashItems = t; }).catch(() => {}) : null])
  .then(() => load())
  .then(() => { if (tab !== "all" || !items.length) setTab(tab, "none"); })
  .catch((e) => console.error(e))
  .finally(() => {
    invoke("window_ready").catch(() => {});
    q.focus();
    requestAnimationFrame(() => {
      document.body.classList.remove("pre");
      document.body.classList.add("opening");
      opening = false;
      if (tab !== "canvases") replayEnter();
      setTimeout(() => {
        document.body.classList.remove("opening");
        document.querySelectorAll<HTMLElement>("#rail > *").forEach((e) => e.style.removeProperty("--i"));
      }, 1500);
    });
  });
