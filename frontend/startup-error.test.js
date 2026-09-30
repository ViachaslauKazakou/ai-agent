import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

test("startup errors display when module loading fails before app.js executes", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  assert.match(html, /<script src="\.\/startup-error\.js"><\/script>\s*<script type="module" src="\.\/app\.js"><\/script>/);
  assert.doesNotMatch(html, /<script(?:\s[^>]*)?>\s*[^\s<]|\sonerror\s*=/i);
  const listeners = new Map();
  const status = { textContent: "Starting service…" };
  const window = { addEventListener: (name, callback) => listeners.set(name, callback) };
  runInNewContext(readFileSync(new URL("./startup-error.js", import.meta.url), "utf8"), {
    window,
    document: { querySelector: (selector) => selector === "#status" ? status : null },
  });
  listeners.get("error")({ message: "failed to load app.js" });
  assert.equal(status.textContent, "Frontend error: failed to load app.js");
  listeners.get("unhandledrejection")({ reason: "import failed" });
  assert.equal(status.textContent, "Frontend error: import failed");
});
