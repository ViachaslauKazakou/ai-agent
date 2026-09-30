import test from "node:test";
import assert from "node:assert/strict";
import { createCoderUI } from "./coder-ui.js";

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

function fixture(invoke) {
  const elements = new Map();
  function element() {
    return {
      children: [], hidden: false, disabled: false, textContent: "", value: "false", listeners: {},
      replaceChildren() { this.children = []; },
      append(child) { this.children.push(child); },
      addEventListener(name, handler) { this.listeners[name] = handler; },
      click() { return this.listeners.click?.(); },
    };
  }
  const previous = globalThis.document;
  globalThis.document = {
    querySelector(selector) {
      if (!elements.has(selector)) elements.set(selector, element());
      return elements.get(selector);
    },
    createElement: element,
  };
  let project = { id: "one" };
  const ui = createCoderUI({ invoke, getProject: () => project });
  return { ui, get: (selector) => elements.get(selector), setProject: (next) => { project = next; },
    restore: () => { globalThis.document = previous; } };
}

test("Coder mode and project transitions clear stale content and gate controls", async () => {
  const state = fixture(async (command) => ({
    coder_tree: { files: [{ path: "code.rs", directory: false }], truncated: false },
    coder_changes: [{ path: "code.rs", status: " M" }],
    coder_venv: { present: false, interpreter: null, note: "Inspection only" },
  })[command]);
  try {
    assert.equal(state.get("#coder-panel").hidden, true);
    state.ui.setMode("coder");
    assert.equal(state.get("#coder-panel").hidden, false);
    await state.ui.refresh();
    assert.equal(state.get("#coder-changes").children.length, 1);
    state.setProject(undefined);
    state.ui.reset();
    assert.equal(state.get("#coder-panel").hidden, true);
    assert.equal(state.get("#coder-changes").children.length, 0);
    assert.equal(state.get("#coder-check").disabled, true);
  } finally { state.restore(); }
});

test("late diff from earlier staged source cannot replace newer diff", async () => {
  const first = deferred();
  const second = deferred();
  const state = fixture(async (command, args) => {
    if (command === "coder_diff") return (args.staged ? second : first).promise;
    return ({ coder_tree: { files: [], truncated: false }, coder_changes: [{ path: "code.rs", status: " M" }],
      coder_venv: { present: false, note: "Inspection only" } })[command];
  });
  try {
    state.ui.setMode("coder");
    await state.ui.refresh();
    const pending = state.get("#coder-changes").children[0].click();
    state.get("#coder-staged").value = "true";
    const newer = state.get("#coder-staged").listeners.change();
    second.resolve({ content: "staged" });
    await newer;
    first.resolve({ content: "unstaged" });
    await pending;
    assert.equal(state.get("#coder-diff").textContent, "staged");
  } finally { state.restore(); }
});
