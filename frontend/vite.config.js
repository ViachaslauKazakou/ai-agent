import { readFileSync } from "node:fs";
import { defineConfig } from "vite";

const cargoToml = readFileSync(new URL("../Cargo.toml", import.meta.url), "utf8");
const version = cargoToml.match(/^version\s*=\s*"([^"]+)"/m)?.[1] || "unknown";

export default defineConfig({
  define: {
    __APP_VERSION__: JSON.stringify(version),
  },
});
