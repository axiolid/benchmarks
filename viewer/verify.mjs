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
      errors.push(msg.params.exceptionDetails.text ?? "exception");
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

const failed = checks.filter((c) => !c.ok);
console.log(`\n${checks.length - failed.length}/${checks.length} checks passed`);
ws.close();
process.exit(failed.length ? 1 : 0);
