// Settings > Statistics (advanced): what is in the stash and how it grew. All numbers come from the library itself
// (stats.rs reads the sidecars when the tab opens), so nothing is tracked in the background.
// Charts are single-hue (the accent): bars for counts, a one-hue calendar for activity. Every mark has a tooltip
// and every bar carries its number, so nothing depends on colour alone.
import { invoke } from "@tauri-apps/api/core";

interface Stats {
  total: number; bytes: number; notes: number; links: number; images: number; videos: number; audio: number; files: number;
  favorites: number; hidden: number; imported: number; captioned: number; tagged: number; with_ocr: number;
  trash: number; trash_bytes: number; canvases: number; video_secs: number; words: number;
  first: string | null; last: string | null; days: Record<string, number>; hours: number[]; weekdays: number[];
  methods: [string, number][]; tags: [string, number][]; tag_count: number; apps: [string, number][]; sites: [string, number][];
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text) e.textContent = text; // textContent only: tags, apps and sites are user data
  return e;
}
const nf = new Intl.NumberFormat();
const num = (n: number) => nf.format(n);
const plural = (n: number, w: string) => `${num(n)} ${w}${n === 1 ? "" : "s"}`;
function bytes(n: number) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  return `${Math.max(0, Math.round(n / 1e3))} KB`;
}
const pad = (n: number) => String(n).padStart(2, "0");
const dayKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const pct = (a: number, b: number) => (b ? `${Math.round((a / b) * 100)}%` : "–");

/** Current and longest run of consecutive days with at least one save. */
function streaks(days: Record<string, number>): { cur: number; best: number } {
  const keys = Object.keys(days).filter((k) => days[k] > 0).sort();
  let best = 0, run = 0, prev: Date | null = null;
  for (const k of keys) {
    const [y, m, d] = k.split("-").map(Number);
    const t = new Date(y, m - 1, d);
    run = prev && Math.round((t.getTime() - prev.getTime()) / 86400000) === 1 ? run + 1 : 1;
    best = Math.max(best, run);
    prev = t;
  }
  let cur = 0;
  const t = new Date();
  if (!days[dayKey(t)]) t.setDate(t.getDate() - 1); // today not started yet: the streak still counts to yesterday
  while (days[dayKey(t)]) { cur++; t.setDate(t.getDate() - 1); }
  return { cur, best };
}

function tile(label: string, value: string, sub = ""): HTMLElement {
  const t = el("div", "stile");
  t.append(el("div", "sv", value), el("div", "sl", label));
  if (sub) t.append(el("div", "ss", sub));
  return t;
}

/** Horizontal bars: label, bar (length = share of the largest), value. */
function bars(rows: [string, number][], fmt: (n: number) => string = num): HTMLElement {
  const box = el("div", "hbars");
  const max = Math.max(1, ...rows.map((r) => r[1]));
  for (const [label, n] of rows) {
    const r = el("div", "hb");
    r.title = `${label}: ${fmt(n)}`;
    const track = el("div", "tr");
    const fill = el("i");
    fill.style.width = `${Math.max(n ? 1.5 : 0, (n / max) * 100)}%`;
    track.append(fill);
    r.append(el("span", "hl", label), track, el("span", "hv", fmt(n)));
    box.append(r);
  }
  return box;
}

/** Vertical bars over a fixed axis (hours, weekdays): label under every `every`-th bar. */
function columns(values: number[], labels: string[], tip: (i: number) => string, every = 1): HTMLElement {
  const box = el("div", "vbars");
  const max = Math.max(1, ...values);
  values.forEach((v, i) => {
    const c = el("div", "vb");
    c.title = tip(i);
    const bar = el("i");
    bar.style.height = `${(v / max) * 100}%`;
    if (!v) bar.classList.add("z");
    c.append(el("div", "col"), el("span", "", i % every === 0 ? labels[i] : ""));
    c.firstElementChild!.append(bar);
    box.append(c);
  });
  return box;
}

/** The last 53 weeks as a calendar (weeks are columns, Monday on top), shaded by items saved that day. */
function calendar(days: Record<string, number>): HTMLElement {
  const wrap = el("div", "cal");
  const grid = el("div", "calg");
  const months = el("div", "calm");
  const today = new Date(); today.setHours(0, 0, 0, 0);
  const start = new Date(today);
  start.setDate(start.getDate() - ((start.getDay() + 6) % 7) - 52 * 7); // a Monday, 52 weeks before this week's
  const vals = Object.values(days);
  const max = Math.max(1, ...vals);
  // four steps by quartile of the busiest day, so one huge day does not wash everything else out
  const level = (n: number) => (n <= 0 ? 0 : n >= max * 0.75 ? 4 : n >= max * 0.4 ? 3 : n >= max * 0.15 ? 2 : 1);
  let lastMonth = -1;
  for (let w = 0; w < 53; w++) {
    const col = el("div", "calw");
    for (let d = 0; d < 7; d++) {
      const t = new Date(start);
      t.setDate(start.getDate() + w * 7 + d);
      const c = el("i");
      if (t > today) c.classList.add("fut");
      else {
        const n = days[dayKey(t)] ?? 0;
        c.className = `l${level(n)}`;
        c.title = `${t.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" })}: ${n ? plural(n, "item") : "nothing saved"}`;
      }
      col.append(c);
    }
    grid.append(col);
    const first = new Date(start); first.setDate(start.getDate() + w * 7);
    const m = el("span", "", first.getMonth() !== lastMonth && first.getDate() <= 7 ? first.toLocaleDateString(undefined, { month: "short" }) : "");
    if (m.textContent) lastMonth = first.getMonth();
    months.append(m);
  }
  const legend = el("div", "call");
  legend.append(el("span", "", "Less"));
  for (let l = 0; l <= 4; l++) legend.append(el("i", `l${l}`));
  legend.append(el("span", "", "More"));
  wrap.append(months, grid, legend);
  return wrap;
}

function card(title: string, ...body: (HTMLElement | string)[]): HTMLElement {
  const c = el("div", "scard");
  c.append(el("h3", "", title), ...body.map((b) => (typeof b === "string" ? el("div", "hint", b) : b)));
  return c;
}

const root = el("div", "stats");
let loading = false;

export function statsRoot(): HTMLElement { return root; }

export async function loadStats() {
  if (loading) return;
  loading = true;
  if (!root.childElementCount) root.append(el("div", "hint sload", "Counting your stash…"));
  try {
    render(await invoke<Stats>("library_stats"));
  } catch (e) {
    root.replaceChildren(el("div", "hint", `Statistics are unavailable: ${e}`));
  } finally {
    loading = false;
  }
}

function render(s: Stats) {
  root.replaceChildren();
  const head = el("div", "shead");
  const refresh = el("button", "btn", "Refresh");
  refresh.onclick = () => loadStats();
  const since = s.first ? new Date(s.first) : null;
  head.append(el("div", "hint", since ? `Since ${since.toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" })}. Counted from the library folder, nothing is sent anywhere.` : "Nothing saved yet."), refresh);
  root.append(head);
  if (!s.total) return;

  const st = streaks(s.days);
  const now = Date.now();
  const lastDays = (n: number) => Object.entries(s.days).filter(([k]) => { const [y, m, d] = k.split("-").map(Number); return now - new Date(y, m - 1, d).getTime() < n * 86400000; }).reduce((a, [, v]) => a + v, 0);
  const active = Object.values(s.days).filter((v) => v > 0).length;
  const busiest = Object.entries(s.days).sort((a, b) => b[1] - a[1])[0];

  const tiles = el("div", "stiles");
  tiles.append(
    tile("Items", num(s.total), `${bytes(s.bytes)} on disk`),
    tile("This week", num(lastDays(7)), `${num(lastDays(30))} in 30 days`),
    tile("Day streak", plural(st.cur, "day"), `Longest ${plural(st.best, "day")}`),
    tile("Per active day", active ? (s.total / active).toFixed(1) : "0", plural(active, "active day")),
    tile("Tags", num(s.tag_count), `${pct(s.tagged, s.total)} of items tagged`),
    tile("Favourites", num(s.favorites), `${num(s.canvases)} canvas${s.canvases === 1 ? "" : "es"}`),
  );
  root.append(tiles);

  const [by, bm, bd] = busiest ? busiest[0].split("-").map(Number) : [0, 0, 0];
  root.append(card("Activity", calendar(s.days),
    busiest ? `Busiest day: ${new Date(by, bm - 1, bd).toLocaleDateString(undefined, { day: "numeric", month: "long", year: "numeric" })} with ${plural(busiest[1], "item")}.` : ""));

  const kinds: [string, number][] = ([["Pictures", s.images], ["Videos", s.videos], ["Notes", s.notes], ["Links", s.links], ["Audio", s.audio], ["Other files", s.files]] as [string, number][]).filter((k) => k[1] > 0);
  const two = el("div", "stwo");
  two.append(card("What you keep", bars(kinds)), card("How you save", bars(s.methods)));
  root.append(two);

  const HOURS = Array.from({ length: 24 }, (_, h) => new Date(2000, 0, 1, h).toLocaleTimeString(undefined, { hour: "numeric" }));
  const WD = Array.from({ length: 7 }, (_, i) => new Date(2024, 0, 1 + i).toLocaleDateString(undefined, { weekday: "short" })); // 2024-01-01 is a Monday
  const when = el("div", "stwo");
  when.append(
    card("Time of day", columns(s.hours, HOURS, (i) => `${HOURS[i]}: ${plural(s.hours[i], "item")}`, 6)),
    card("Day of the week", columns(s.weekdays, WD, (i) => `${WD[i]}: ${plural(s.weekdays[i], "item")}`)),
  );
  root.append(when);

  const tops = el("div", "stwo");
  if (s.tags.length) tops.append(card("Top tags", bars(s.tags.map(([t, n]) => ["#" + t, n]))));
  if (s.apps.length) tops.append(card("Top apps", bars(s.apps)));
  if (s.sites.length) tops.append(card("Top sites", bars(s.sites)));
  if (tops.childElementCount) root.append(tops);

  const facts = el("div", "facts");
  const fact = (k: string, v: string) => facts.append(el("span", "", k), el("b", "", v));
  fact("Captioned", `${num(s.captioned)} (${pct(s.captioned, s.total)})`);
  fact("Text found in pictures", `${num(s.with_ocr)} of ${num(s.images)}`);
  fact("Words written in notes", num(s.words));
  if (s.video_secs) fact("Recorded video", s.video_secs >= 3600 ? `${(s.video_secs / 3600).toFixed(1)} h` : `${Math.round(s.video_secs / 60)} min`);
  fact("Imported from folders", num(s.imported));
  fact("Hidden", num(s.hidden));
  fact("In the trash", `${num(s.trash)} (${bytes(s.trash_bytes)})`);
  if (s.last) fact("Last saved", new Date(s.last).toLocaleString());
  root.append(card("More", facts));
}
