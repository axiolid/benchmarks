import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import App from "./App";

// Fixture shaped exactly like /api/results, including the real lite_kernel
// failure (null) so the UI is proven to handle a declining kernel.
const FIXTURE = {
  reps: 3,
  mismatches: 1,
  built: ["axiolid", "cellular", "manifold", "cgal", "occt", "lite_kernel"],
  rows: [
    { n: 1, axiolid: 0.11, cellular: 0.004, manifold: 0.09, cgal: 0.64, occt: 3.1, lite_kernel: 2.5 },
    { n: 64, axiolid: 8.1, cellular: 0.3, manifold: 4.6, cgal: 990.0, occt: 917.9, lite_kernel: null },
  ],
};

// recharts' ResponsiveContainer measures its parent; jsdom reports 0x0 and the
// chart then renders nothing. Give elements a real box so the SVG is produced.
beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, value: 900 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, value: 380 });
  Object.defineProperty(HTMLElement.prototype, "getBoundingClientRect", {
    configurable: true,
    value: () => ({ width: 900, height: 380, top: 0, left: 0, bottom: 380, right: 900, x: 0, y: 0 }),
  });
  global.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

afterEach(() => vi.restoreAllMocks());

describe("App", () => {
  it("renders charts from fetched results", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => FIXTURE })));
    const { container } = render(<App />);

    // Heading proves mount; kernel names prove the data reached the view.
    await waitFor(() => expect(screen.getByText(/Axiolid kernel benchmarks/i)).toBeTruthy());
    await waitFor(() => expect(container.querySelectorAll("svg.recharts-surface").length).toBeGreaterThan(0));

    // Assert the DISPLAY labels users actually see, not the JSON keys.
    for (const label of ["axiolid", "cellular (ours)", "Manifold", "CGAL", "OpenCascade"]) {
      expect(
        screen.getAllByText((_t, el) => (el?.textContent ?? "").includes(label)).length,
      ).toBeGreaterThan(0);
    }
  });

  it("surfaces a failing kernel instead of hiding it", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: true, json: async () => FIXTURE })));
    render(<App />);
    // 1 mismatch is the known-good lite-kernel zero-volume failure.
    // Text is split across JSX nodes and several ancestors match, so assert on
    // the innermost matching element rather than a unique-match query.
    await waitFor(() => {
      const hits = screen.getAllByText((_t, el) =>
        /^1 volume mismatch$/i.test((el?.textContent ?? "").trim()),
      );
      expect(hits.length).toBeGreaterThan(0);
    });
  });

  it("shows an error state when the API fails", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 500, text: async () => "boom" })));
    render(<App />);
    await waitFor(() => expect(screen.getByText(/failed|error/i)).toBeTruthy());
  });
});
