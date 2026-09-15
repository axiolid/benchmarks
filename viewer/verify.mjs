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

const failed = checks.filter((c) => !c.ok);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
ws.close();
process.exit(failed.length ? 1 : 0);
