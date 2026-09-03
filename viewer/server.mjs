// Benchmark viewer server.
//
// Two jobs: serve the built SPA, and expose the harness's own `--json` output
// at /api/results. It never computes or massages numbers itself -- it shells
// out to the release binary and passes the payload through, so what the UI
// draws is exactly what the harness measured.
import { spawn } from "node:child_process";
import { readFile, stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, join, normalize, resolve } from "node:path";

const PORT = Number(process.env.PORT ?? 8095);
const HOST = process.env.HOST ?? "127.0.0.1";
const ROOT = resolve(import.meta.dirname);
const DIST = join(ROOT, "dist");
const CRATE = resolve(ROOT, "..");

// A run is seconds of CPU work, so results are cached and only recomputed when
// the client explicitly asks (?fresh=1) or reps change.
const cache = new Map();

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".svg": "image/svg+xml",
  ".json": "application/json; charset=utf-8",
  ".woff2": "font/woff2",
};

/** Run the harness and parse its JSON. Rejects on non-zero exit or bad JSON. */
function runBenchmark(reps) {
  return new Promise((ok, fail) => {
    const child = spawn(
      "cargo",
      ["run", "--release", "--quiet", "--", String(reps), "--json"],
      {
        cwd: CRATE,
        env: {
          ...process.env,
          CARGO_TARGET_DIR: "/mnt/backup/build-cache/kernel-bench-target",
          CARGO_HOME: "/mnt/backup/build-cache/axiolid-rayon-cargo-home",
        },
      },
    );
    let out = "";
    let err = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (err += d));
    child.on("error", fail);
    child.on("close", (code) => {
      if (code !== 0) return fail(new Error(`harness exited ${code}: ${err.slice(-400)}`));
      try {
        // stderr carries the volume-mismatch warnings; surface them so the UI
        // can show that a column is untrustworthy rather than hiding it.
        const parsed = JSON.parse(out);
        parsed.warnings = err
          .split("\n")
          .map((l) => l.trim())
          .filter((l) => l.startsWith("!!"));
        ok(parsed);
      } catch (e) {
        fail(new Error(`bad JSON from harness: ${e.message}`));
      }
    });
  });
}

async function serveStatic(req, res, pathname) {
  // normalize + prefix check: prevents ../ escaping out of dist.
  const target = join(DIST, normalize(pathname === "/" ? "/index.html" : pathname));
  if (!target.startsWith(DIST)) {
    res.writeHead(403).end("forbidden");
    return;
  }
  try {
    const info = await stat(target);
    if (!info.isFile()) throw new Error("not a file");
    const body = await readFile(target);
    res.writeHead(200, {
      "content-type": TYPES[extname(target)] ?? "application/octet-stream",
      "cache-control": target.includes("/assets/") ? "public, max-age=31536000, immutable" : "no-cache",
    });
    res.end(body);
  } catch {
    // SPA fallback, but only for navigations -- a missing asset must 404 rather
    // than silently return HTML, which turns into a confusing MIME error.
    if (req.headers.accept?.includes("text/html")) {
      try {
        res.writeHead(200, { "content-type": TYPES[".html"] });
        res.end(await readFile(join(DIST, "index.html")));
        return;
      } catch {
        /* fall through */
      }
    }
    res.writeHead(404).end("not found");
  }
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "localhost"}`);

  if (url.pathname === "/api/health") {
    res.writeHead(200, { "content-type": TYPES[".json"] });
    res.end(JSON.stringify({ ok: true }));
    return;
  }

  if (url.pathname === "/api/results") {
    const reps = Math.min(Math.max(Number(url.searchParams.get("reps") ?? 5) || 5, 1), 25);
    const fresh = url.searchParams.get("fresh") === "1";
    try {
      if (fresh || !cache.has(reps)) {
        cache.set(reps, await runBenchmark(reps));
      }
      res.writeHead(200, { "content-type": TYPES[".json"], "cache-control": "no-store" });
      res.end(JSON.stringify({ ...cache.get(reps), cached: !fresh && cache.has(reps) }));
    } catch (e) {
      // Report the failure instead of serving stale or invented numbers.
      res.writeHead(500, { "content-type": TYPES[".json"] });
      res.end(JSON.stringify({ error: String(e.message ?? e) }));
    }
    return;
  }

  await serveStatic(req, res, url.pathname);
});

server.listen(PORT, HOST, () => {
  console.log(`benchmarks viewer on http://${HOST}:${PORT}`);
});
