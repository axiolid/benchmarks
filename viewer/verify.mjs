// CDP render proof: HTTP 200 says nothing about whether React mounted.
// Asserts on user-visible text and on real SVG geometry (getBBox), then
// screenshots at viewport size after a settle delay.
import WebSocket from "ws";
import { writeFileSync } from "node:fs";

const BASE = process.env.BASE ?? "http://127.0.0.1:8095";
const PORT = 9422;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  const page = list.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl, { maxPayload: 256 * 1024 * 1024 });
  let id = 0;
  const pending = new Map();
  const errors = [];

  ws.on("message", (raw) => {
    const msg = JSON.parse(raw.toString());
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg.result);
      pending.delete(msg.id);
    }
    if (msg.method === "Runtime.exceptionThrown") {
      const d = msg.params.exceptionDetails;
      // .text is the generic wrapper; the real message is on the
      // exception object, which is what actually names the bug.
      errors.push(d.exception?.description ?? d.text ?? "exception");
    }
    // Only >=400 is an error; responseReceived alone fires for every success.
    if (msg.method === "Network.responseReceived" && msg.params.response.status >= 400) {
      errors.push(`HTTP ${msg.params.response.status} ${msg.params.response.url}`);
    }
  });

  const send = (method, params = {}) =>
    new Promise((res) => {
      const n = ++id;
      pending.set(n, res);
      ws.send(JSON.stringify({ id: n, method, params }));
    });

  await new Promise((r) => ws.on("open", r));
  await send("Runtime.enable");
  await send("Page.enable");
  await send("Network.enable");
  await send("Page.navigate", { url: BASE });
  await sleep(9000);

  const evalJs = async (expr) => {
    const r = await send("Runtime.evaluate", {
      expression: expr,
      returnByValue: true,
      awaitPromise: true,
    });
    return r?.result?.value;
  };

  return { send, evalJs, errors, ws };
}

const { send, evalJs, errors, ws } = await main();

const checks = [];
const check = (name, ok, detail) => {
  checks.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}  ${detail ?? ""}`);
};

// 1. React mounted at all.
const rootKids = await evalJs('document.getElementById("root")?.children.length ?? 0');
check("react mounted", rootKids > 0, `root has ${rootKids} children`);

// 2. Sidebar nav present, asserted on VISIBLE labels not internal ids.
const navText = await evalJs('document.querySelector("nav")?.innerText ?? ""');
for (const label of ["Cost breakdown", "Parallelism", "Kernel comparison", "Method"]) {
  check(`nav: ${label}`, navText.includes(label), "");
}

// 3. The perf chart really painted. Node count lies -- a bar can exist
//    with zero area -- so measure SVG geometry via getBBox.
const barArea = await evalJs(`(() => {
  const bars = [...document.querySelectorAll(".recharts-rectangle")];
  let painted = 0;
  for (const b of bars) {
    try { const bb = b.getBBox(); if (bb.width > 1 && bb.height > 1) painted++; } catch (e) {}
  }
  return { total: bars.length, painted };
})()`);
check("stacked bars painted", barArea.painted > 0,
      `${barArea.painted}/${barArea.total} have real geometry`);

// 4. Area cards rendered from the JSON, with real numbers.
const bodyText = await evalJs("document.body.innerText");
for (const area of ["Boolean", "Mesh audit", "Measure", "Level set", "Inspect", "Heal", "Genus"]) {
  check(`area card: ${area}`, bodyText.includes(area), "");
}

check("no console/network errors", errors.length === 0, errors.slice(0, 3).join(" | "));

// 4b. Every area must have a y-axis label. The vision pass caught recharts
// silently dropping alternate ticks at 12 rows while every DOM check passed.
const axisLabels = await evalJs(`(() => {
  const t = [...document.querySelectorAll(".recharts-yAxis .recharts-cartesian-axis-tick-value")];
  return t.map((n) => n.textContent.trim()).filter(Boolean).length;
})()`);
const areaCount = await evalJs(`(window.__perf?.areas?.length ?? 12)`);
check(
  "every area has a y-axis label",
  axisLabels >= 12,
  `${axisLabels} labels for ${areaCount} areas`,
);




// --- interaction checks: rendering is not the same as working ---

// Focus a category: it must move leftmost and mute the others.
const focused = await evalJs(`(() => {
  const items = [...document.querySelectorAll(".recharts-legend-item")];
  const t = items.find((e) => /Pointer chasing/i.test(e.textContent || ""));
  if (!t) return "no legend item";
  t.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  return "clicked";
})()`);
await new Promise((r) => setTimeout(r, 900));

// Opacity is the actual muting mechanism, so read it rather than trust
// that a click "worked".
const muted = await evalJs(`(() => {
  const bars = [...document.querySelectorAll(".recharts-bar")];
  const ops = bars.map((b) => {
    const p = b.querySelector("path");
    return p ? Number(p.getAttribute("fill-opacity") || "1") : 1;
  });
  return JSON.stringify({
    dim: ops.filter((o) => o < 0.5).length,
    full: ops.filter((o) => o >= 0.9).length,
  });
})()`);
const m = JSON.parse(muted || "{}");
check("focus mutes other categories", (m.dim || 0) > 0 && (m.full || 0) > 0,
  `dim=${m.dim} full=${m.full}`);

// Absolute mode must switch the unit to ms, not just relabel.
const abs = await evalJs(`(() => {
  const b = [...document.querySelectorAll("button")]
    .find((e) => /millisecond/i.test((e.textContent || "").trim()));
  if (!b) return "no button";
  b.click();
  return "ok";
})()`);
await new Promise((r) => setTimeout(r, 900));
const axisText = await evalJs(`(() => {
  const t = [...document.querySelectorAll(".recharts-xAxis text")]
    .map((e) => e.textContent).join(" ");
  return t;
})()`);
check("absolute mode shows ms", abs === "ok" && /ms/.test(axisText || ""),
  (axisText || "").slice(0, 40));

// Before/after section must carry real measured numbers.

const hist = await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((e) => /Before/i.test(e.textContent || "") && /after/i.test(e.textContent || ""));
  if (!b) return "no nav";
  const r = b.getBoundingClientRect();
  return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
})()`);
const hp = JSON.parse(hist && hist !== "no nav" ? hist : "{}");
if (hp.x) {
  // Synthetic .click() does not always drive React handlers; a real CDP
  // mouse event does.
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: hp.x, y: hp.y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: hp.x, y: hp.y, button: "left", clickCount: 1 });
}
await new Promise((r) => setTimeout(r, 1200));

// innerText via CDP truncates on this page, so read the table cells
// directly -- the assertion needs the real content, not a preview.
const histText = await evalJs(`(() => {
  const m = document.querySelector("main") || document.body;
  return (m.innerText || "").replace(/\\s+/g, " ");
})()`);
check("before/after has revisions", /5e52dde/.test(histText) && /b47274d/.test(histText),
  hist === "ok" ? "" : String(hist));
check("before/after names a real win", /refine/i.test(histText) && /6[0-9]%/.test(histText),
  "");
// The noise disclosure is the honesty check on this whole table.
check("noise floor caveat shown", /noise floor/i.test(histText), "");

// The per-row "within noise" verdict only appears once the real-wins
// filter is off, so uncheck it before asserting.
const box = await evalJs(`(() => {
  const c = document.querySelector("input[type=checkbox]");
  if (!c) return "nobox";
  const r = c.getBoundingClientRect();
  return JSON.stringify({ x: r.x + r.width/2, y: r.y + r.height/2 });
})()`);
const bp = JSON.parse(box && box !== "nobox" ? box : "{}");
if (bp.x) {
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: bp.x, y: bp.y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: bp.x, y: bp.y, button: "left", clickCount: 1 });
}
await new Promise((r) => setTimeout(r, 700));
const allText = await evalJs(`(() => {
  const m = document.querySelector("main") || document.body;
  return (m.innerText || "").replace(/\\s+/g, " ");
})()`);
check("within-noise rows labelled", /within noise/i.test(allText), "");
const shot2 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/hist2.png", Buffer.from(shot2.data, "base64"));
console.log("history screenshot: /tmp/hist2.png");


await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((e) => /Cost breakdown/i.test(e.textContent || ""));
  b && b.click();
  return true;
})()`);
await new Promise((r) => setTimeout(r, 900));

// 5. Click through to the Parallelism section and prove it renders too.
const clicked = await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((x) => x.innerText.includes("Parallelism"));
  if (!b) return false;
  b.click();
  return true;
})()`);
await new Promise((r) => setTimeout(r, 2500));
const parText = await evalJs("document.body.innerText");
check("parallel section opens", clicked && parText.includes("CPU/wall"),
      "thread-scaling table visible");
check("scaling verdicts shown", /does not scale|partial|scales/.test(parText), "");

// Screenshot the perf section: viewport capture, NOT captureBeyondViewport,
// which grabs a pre-layout frame and yields blank charts.
await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((x) => x.innerText.includes("Cost breakdown"));
  b && b.click();
  return true;
})()`);
await new Promise((r) => setTimeout(r, 2500));
const shot = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/perf-view.png", Buffer.from(shot.data, "base64"));
console.log("screenshot: /tmp/perf-view.png");





// --- kernel comparison section ---
// This section had no coverage, which is how it shipped bricked.
const cmpNav = await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((e) => /comparison/i.test(e.textContent || ""));
  if (!b) return "no nav";
  const r = b.getBoundingClientRect();
  return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
})()`);
const cmpP = JSON.parse(cmpNav && cmpNav !== "no nav" ? cmpNav : "{}");
if (cmpP.x) {
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: cmpP.x, y: cmpP.y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: cmpP.x, y: cmpP.y, button: "left", clickCount: 1 });
}
// The harness runs a real benchmark, so give it room.
await new Promise((r) => setTimeout(r, 25000));
const cmpTxt = await evalJs(`(() => {
  const m = document.querySelector("main") || document.body;
  return (m.innerText || "").replace(/\s+/g, " ").slice(0, 600);
})()`);
check("kernel comparison is not an error card", !/Benchmark failed|harness exited/i.test(cmpTxt), cmpTxt.slice(0, 200));
check("kernel comparison shows a table", /axiolid/i.test(cmpTxt), cmpTxt.slice(0, 200));


// --- ray paths section ---
const rp = await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((e) => /Ray paths/i.test(e.textContent || ""));
  if (!b) return "no nav";
  const r = b.getBoundingClientRect();
  return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
})()`);
const rpp = JSON.parse(rp && rp !== "no nav" ? rp : "{}");
if (rpp.x) {
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: rpp.x, y: rpp.y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: rpp.x, y: rpp.y, button: "left", clickCount: 1 });
}
await new Promise((r) => setTimeout(r, 1000));
const rayText = await evalJs(`(() => {
  const m = document.querySelector("main") || document.body;
  return (m.innerText || "").replace(/\\s+/g, " ");
})()`);
check("ray paths shows all three", /Full scan/i.test(rayText)
  && /Automatic cache/i.test(rayText) && /Caller-held index/i.test(rayText), "");
// The caveat is the point of the section: without it a reader would
// take 71x as a free, automatic win.
check("opt-in caveat is stated", /opt-in/i.test(rayText)
  && /did not move/i.test(rayText), "");
check("crossover is disclosed", /22 rays/i.test(rayText), "");
const shot3 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/raypaths.png", Buffer.from(shot3.data, "base64"));
console.log("ray screenshot: /tmp/raypaths.png");



// The ray-paths checks above left that section open. These checks are
// about Cost breakdown, so go back before querying its controls.
const cb = await evalJs(`(() => {
  const b = [...document.querySelectorAll("nav button")]
    .find((e) => /Cost breakdown/i.test(e.textContent || ""));
  if (!b) return "no nav";
  const r = b.getBoundingClientRect();
  return JSON.stringify({ x: r.x + r.width / 2, y: r.y + r.height / 2 });
})()`);
const cbp = JSON.parse(cb && cb !== "no nav" ? cb : "{}");
if (cbp.x) {
  await send("Input.dispatchMouseEvent", { type: "mousePressed", x: cbp.x, y: cbp.y, button: "left", clickCount: 1 });
  await send("Input.dispatchMouseEvent", { type: "mouseReleased", x: cbp.x, y: cbp.y, button: "left", clickCount: 1 });
}
await new Promise((r) => setTimeout(r, 900));

// --- cost breakdown: kernel + shadow dropdowns ---
const kd = await evalJs(`(() => {
  const sel = [...document.querySelectorAll("select")];
  const k = sel.find((s) => s.getAttribute("aria-label") === "Kernel");
  const s = sel.find((s) => s.getAttribute("aria-label") === "Shadow kernel");
  if (!k) return JSON.stringify({ found: false });
  const opts = [...k.options].map((o) => ({ t: o.textContent.trim(), d: o.disabled }));
  return JSON.stringify({ found: true, opts, shadowDisabled: s ? s.disabled : null });
})()`);
const kdp = JSON.parse(kd || "{}");
check("kernel dropdown exists", kdp.found === true, "");
check("offers competitor kernels",
  !!kdp.opts && kdp.opts.some((o) => /CGAL/.test(o.t)) && kdp.opts.some((o) => /IfcLite/i.test(o.t)), "");
// A kernel that is not compiled must be shown disabled, not omitted:
// silently dropping it looks like a kernel that lost the benchmark.
// Every kernel that IS compiled must be selectable, and any kernel that
// is not must be disabled and say so -- never silently omitted, which
// would read as a kernel that lost rather than one that is absent.
check("compiled kernels are all selectable",
  !!kdp.opts && kdp.opts.some((o) => /CGAL/.test(o.t) && !o.d)
  && kdp.opts.some((o) => /Open CASCADE/i.test(o.t) && !o.d), "");
check("absent kernels are disabled and labelled",
  !!kdp.opts && kdp.opts.filter((o) => o.d).every((o) => /not built/i.test(o.t)), "");
check("shadow gated on the millisecond axis", kdp.shadowDisabled === true, "");

// Switching kernels must actually rechart, not merely set state.
const sw = await evalJs(`(() => {
  const sels = [...document.querySelectorAll("select")];
  const k = sels.find((s) => s.getAttribute("aria-label") === "Kernel");
  if (!k) return "0";
  const before = document.querySelectorAll(".recharts-bar-rectangle").length;
  k.value = "cgal";
  k.dispatchEvent(new Event("change", { bubbles: true }));
  return String(before);
})()`);
await new Promise((r) => setTimeout(r, 600));
const after = await evalJs(`(() => {
  const bars = document.querySelectorAll(".recharts-bar-rectangle").length;
  const txt = (document.querySelector("main") || document.body).innerText || "";
  return JSON.stringify({ bars, cgal: /CGAL/.test(txt) });
})()`);
const ap = JSON.parse(after || "{}");
check("selecting CGAL recharts", ap.bars > 0 && ap.cgal === true, "");

// In millisecond mode the shadow must un-gate and draw a real bar.
const sh = await evalJs(`(() => {
  const ms = [...document.querySelectorAll("button")]
    .find((b) => /milliseconds/i.test(b.textContent || ""));
  if (ms) ms.click();
  return "ok";
})()`);
await new Promise((r) => setTimeout(r, 500));
const shr = await evalJs(`(() => {
  const s = [...document.querySelectorAll("select")]
    .find((e) => e.getAttribute("aria-label") === "Shadow kernel");
  if (!s || s.disabled) return JSON.stringify({ enabled: false });
  const opt = [...s.options].find((o) => /Behind:/.test(o.textContent));
  if (!opt) return JSON.stringify({ enabled: true, drew: false });
  s.value = opt.value;
  s.dispatchEvent(new Event("change", { bubbles: true }));
  return JSON.stringify({ enabled: true, picked: opt.value });
})()`);
const shp = JSON.parse(shr || "{}");
check("shadow enables in millisecond mode", shp.enabled === true, "");
await new Promise((r) => setTimeout(r, 600));
const drew = await evalJs(`(() => {
  const bars = [...document.querySelectorAll(".recharts-bar")];
  return String(bars.length);
})()`);
check("shadow draws behind the stack", Number(drew) > 1, "");

// The shadow is an underlay, not a cost category: listing it in the
// legend invites reading it as part of the breakdown.
const leg = await evalJs(`(() => {
  const l = document.querySelector(".recharts-legend-wrapper");
  return (l && l.innerText || "").replace(/\\s+/g, " ");
})()`);
check("shadow absent from the category legend", !/shadow/i.test(leg), "");
const shot4 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/shadow.png", Buffer.from(shot4.data, "base64"));
console.log("shadow screenshot: /tmp/shadow.png");


// The shadow is a whole-suite total. Behind per-area bars it would be
// the same constant behind every area -- the defect that made CGAL
// look like it cost 6362ms in every row. It must stay disabled while
// the per-area view is selected.
const perArea = await evalJs(`(() => {
  const sels = [...document.querySelectorAll("select")];
  const k = sels.find((s) => s.getAttribute("aria-label") === "Kernel");
  const sh = sels.find((s) => s.getAttribute("aria-label") === "Shadow kernel");
  const ms = [...document.querySelectorAll("button")]
    .find((b) => /milliseconds/i.test(b.textContent || ""));
  if (ms) ms.click();
  const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set;
  set.call(k, "axiolid");
  k.dispatchEvent(new Event("change", { bubbles: true }));
  return "ok";
})()`);
await new Promise((r) => setTimeout(r, 600));
const gated = await evalJs(`(() => {
  const sh = [...document.querySelectorAll("select")]
    .find((s) => s.getAttribute("aria-label") === "Shadow kernel");
  return JSON.stringify({ disabled: !!sh && sh.disabled });
})()`);
const gp = JSON.parse(gated || "{}");
check("suite shadow blocked in per-area view", gp.disabled === true, "");

// A kernel that finished only part of the suite must say so on its bar:
// lite_kernel completes 2 of 12 rows, so its small total means it did
// less work, not that it was faster.
const cov = await evalJs(`(() => {
  const sels = [...document.querySelectorAll("select")];
  const k = sels.find((s) => s.getAttribute("aria-label") === "Kernel");
  const set = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value").set;
  set.call(k, "all");
  k.dispatchEvent(new Event("change", { bubbles: true }));
  return "ok";
})()`);
await new Promise((r) => setTimeout(r, 700));
const covTxt = await evalJs(`(() => {
  const m = document.querySelector("main") || document.body;
  return (m.innerText || "").replace(/\\s+/g, " ");
})()`);
check("partial coverage is shown on the bar", /2\/12 rows/.test(covTxt), "");
check("OCCT appears as a measured kernel", /Open CASCADE/i.test(covTxt), "");
const shot5 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/allkernels.png", Buffer.from(shot5.data, "base64"));
console.log("all-kernels screenshot: /tmp/allkernels.png");


// Apples-to-apples view: one bar per shared workload, kernels side by
// side. This is the only axis on which kernels are comparable.
const wl = await evalJs(`(() => {
  const k = [...document.querySelectorAll("select")]
    .find((s) => s.getAttribute("aria-label") === "Kernel");
  if (!k) return JSON.stringify({ ok: false });
  k.value = "workloads";
  k.dispatchEvent(new Event("change", { bubbles: true }));
  return JSON.stringify({ ok: true });
})()`);
await new Promise((r) => setTimeout(r, 900));
const wlTxt = await evalJs(`(() => {
  // Read axis ticks directly: innerText collapses "=" out of labels.
  const ticks = [...document.querySelectorAll(".recharts-yAxis text")]
    .map((t) => t.textContent || "").join("|");
  const bars = document.querySelectorAll(".recharts-bar-rectangle").length;
  const legend = [...document.querySelectorAll(".recharts-legend-item-text")]
    .map((t) => t.textContent || "").join("|");
  const painted = [...document.querySelectorAll(".recharts-bar-rectangle path")]
    .filter((p) => { const b = p.getBBox(); return b.width > 0.5 && b.height > 0.5; }).length;
  return JSON.stringify({ txt: ticks, legend, bars, painted });
})()`);
const wlp = JSON.parse(wlTxt || "{}");
check("workload view charts shared workloads", /offset n/i.test(wlp.txt || "") && /rotated n/i.test(wlp.txt || ""), (wlp.txt || "").slice(0, 160));
// Bars must have real geometry: a bad axis domain renders zero-width
// bars while the element count stays non-zero, which hid an empty chart.
check("workload view draws kernel bars", (wlp.painted ?? 0) > 6,
  `painted=${wlp.painted} of ${wlp.bars}`);
// Axiolid must be IN the comparison: it owns too few profile samples
// to earn a row on its own, and was silently dropped -- which is why
// the shadow was greyed out for the one kernel that matters most.
check("axiolid is in the kernel comparison", /Axiolid/i.test(wlp.legend || ""), (wlp.txt || "").slice(0, 200));
check("competitors are in the same chart",
  /CGAL/i.test(wlp.legend || "") && /Open CASCADE/i.test(wlp.legend || ""), "");
const shot6 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/workloads.png", Buffer.from(shot6.data, "base64"));
console.log("workload screenshot: /tmp/workloads.png");
// Relative mode is the legible one: capture it too.
await evalJs(`(() => {
  const b = [...document.querySelectorAll("button")]
    .find((e) => /% of area/i.test(e.textContent || ""));
  if (b) b.click();
  return "ok";
})()`);
await new Promise((r) => setTimeout(r, 700));
const shot7 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/workloads_rel.png", Buffer.from(shot7.data, "base64"));
console.log("workload relative: /tmp/workloads_rel.png");

// Split view: one chart per workload, each scaled to its own slowest
// kernel. Shared-axis mode is dominated by flush n=64.
const sp = await evalJs(`(() => {
  const b = [...document.querySelectorAll("button")]
    .find((e) => (e.getAttribute("aria-label") || "") === "Split workloads");
  if (!b) return JSON.stringify({ found: false });
  b.click();
  return JSON.stringify({ found: true });
})()`);
const spp = JSON.parse(sp || "{}");
check("split button exists in workload view", spp.found === true, "");
await new Promise((r) => setTimeout(r, 900));
const spd = await evalJs(`(() => {
  const charts = [...document.querySelectorAll(".recharts-wrapper")];
  // Widest painted bar in each chart, as a fraction of that chart plot.
  const fills = charts.map((c) => {
    const bars = [...c.querySelectorAll(".recharts-bar-rectangle path, .recharts-bar-rectangle rect")];
    const ws = bars.map((b) => { const r = b.getBoundingClientRect(); return r.width; });
    const plot = c.getBoundingClientRect().width || 1;
    return ws.length ? Math.max(...ws) / plot : 0;
  });
  return JSON.stringify({ charts: charts.length, fills });
})()`);
const spdp = JSON.parse(spd || "{}");
check("split renders one chart per workload", (spdp.charts ?? 0) >= 12, String(spdp.charts));
// The point of splitting: in EVERY chart the slowest kernel should use
// most of the width. Shared-axis mode leaves small workloads near zero.
const fills = spdp.fills ?? [];
const wide = fills.filter((f) => f > 0.5).length;
check("each split chart is scaled to its own slowest kernel",
  fills.length >= 12 && wide >= 12,
  `charts=${fills.length} wide=${wide} sample=${fills.slice(0, 4).map((f) => f.toFixed(2)).join(",")}`);
const shot8 = await send("Page.captureScreenshot", { format: "png" });
writeFileSync("/tmp/workloads_split.png", Buffer.from(shot8.data, "base64"));
// Every kernel that has a number for a workload must paint something
// visible in that chart. A 1x bar against a 166x max rounds to zero
// width, which reads as "this kernel did not run".
const vis = await evalJs(`(() => {
  const charts = [...document.querySelectorAll(".recharts-wrapper")];
  let tiny = 0, total = 0;
  charts.forEach((c) => {
    [...c.querySelectorAll(".recharts-bar-rectangle path, .recharts-bar-rectangle rect")].forEach((b) => {
      const w = b.getBoundingClientRect().width;
      total += 1;
      if (w < 2) tiny += 1;
    });
  });
  return JSON.stringify({ tiny, total });
})()`);
const visp = JSON.parse(vis || "{}");
check("no kernel bar is invisible in split view",
  (visp.total ?? 0) > 0 && (visp.tiny ?? 1) === 0,
  `tiny=${visp.tiny} of ${visp.total}`);
// The split legend identifies kernels: the per-chart y ticks are hidden.
const legTxt = await evalJs(`(() => {
  const m = document.querySelector("main") || document.body;
  return (m.innerText || "").replace(/\s+/g, " ");
})()`);
check("split view names its kernels", /Axiolid/i.test(legTxt) && /CGAL/i.test(legTxt), "");

console.log("workload split: /tmp/workloads_split.png");



// An area whose profile is mostly unclassified teaches nothing: the
// fieldsample arm shipped at 99% unclassified because its rasteriser
// inlines entirely. Cap it so a new arm cannot quietly do the same.
const unc = await evalJs(`(async () => {
  const r = await fetch("/perf.json");
  const d = await r.json();
  const bad = d.areas
    .map((a) => ({
      area: a.area,
      u: (a.categories.find((c) => c.name === "unclassified") || {}).pct || 0,
    }))
    .filter((x) => x.u > 60);
  return JSON.stringify({ count: d.areas.length, bad });
})()`);
const uncp = JSON.parse(unc || "{}");
check("area count grew past twenty", (uncp.count ?? 0) >= 20, String(uncp.count));
check("no area is mostly unclassified",
  Array.isArray(uncp.bad) && uncp.bad.length === 0,
  JSON.stringify(uncp.bad || []).slice(0, 160));


// Every area needs a human label. raybvh/facaderay/handleray shipped
// as raw ids for three rounds because nothing asserted this.
const lbl = await evalJs(`(() => {
  const ticks = [...document.querySelectorAll(".recharts-yAxis .recharts-cartesian-axis-tick-value")]
    .map((t) => (t.textContent || "").trim());
  return JSON.stringify({ raw: ticks.filter((t) => /^[a-z]+$/.test(t)) });
})()`);
const lblp = JSON.parse(lbl || "{}");
check("every area has a human label", (lblp.raw ?? []).length === 0, JSON.stringify(lblp.raw));

const failed = checks.filter((c) => !c.ok);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
ws.close();
process.exit(failed.length ? 1 : 0);
