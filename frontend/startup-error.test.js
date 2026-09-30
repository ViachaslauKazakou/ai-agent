import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

test("startup errors display when module loading fails before app.js executes", () => {
  const html = readFileSync(new URL("./index.html", import.meta.url), "utf8");
  assert.ok(html.indexOf('<script src="/startup-error.js">') >= 0);
  assert.ok(html.indexOf('<script src="/startup-error.js">') < html.indexOf('<script type="module" src="./app.js">'));
  assert.doesNotMatch(html, /<script(?:\s[^>]*)?>\s*[^\s<]|\sonerror\s*=/i);
  const listeners = new Map();
  const status = { textContent: "Starting service…" };
  const window = { addEventListener: (name, callback) => listeners.set(name, callback) };
  runInNewContext(readFileSync(new URL("./public/startup-error.js", import.meta.url), "utf8"), {
    window,
    document: { querySelector: (selector) => selector === "#status" ? status : null },
  });
  listeners.get("error")({ message: "failed to load app.js" });
  assert.equal(status.textContent, "Frontend error: failed to load app.js");
  listeners.get("unhandledrejection")({ reason: "import failed" });
  assert.equal(status.textContent, "Frontend error: import failed");
});

test("production HTML keeps the standalone startup handler before its module", () => {
  const html = readFileSync(new URL("./dist/index.html", import.meta.url), "utf8");
  const startup = readFileSync(new URL("./dist/startup-error.js", import.meta.url), "utf8");
  assert.match(html, /<script src="\/startup-error\.js"><\/script>/);
  assert.ok(html.indexOf('src="/startup-error.js"') < html.indexOf('type="module"'));
  assert.doesNotMatch(html, /<script(?:\s[^>]*)?>\s*[^\s<]|\son\w+\s*=/i);
  assert.match(startup, /addEventListener\("error"/);
  assert.match(startup, /addEventListener\("unhandledrejection"/);
});
