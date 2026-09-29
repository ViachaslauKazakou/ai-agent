import { readFileSync } from "node:fs";
import { defineConfig } from "vite";

const cargoToml = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8");
const version = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1] || "unknown";

export default defineConfig({
  // Tauri expects this exact URL. Falling back to another port would leave the
  // webview waiting on 1420 while Vite serves an unrelated address.
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  define: {
    __APP_VERSION__: JSON.stringify(version),
  },
});
