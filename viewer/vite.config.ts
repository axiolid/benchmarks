import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { "@": resolve(import.meta.dirname, "src") } },
  server: {
    port: 5190,
    strictPort: true,
    // Tailscale serves this over HTTPS on a *.ts.net host; Vite blocks unknown
    // Host headers by default, so the service domain must be allowed.
    allowedHosts: [".ts.net"],
    proxy: { "/api": "http://127.0.0.1:8095" },
  },
});
