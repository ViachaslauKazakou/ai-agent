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
      append(child) { this.children.push(child); if (this.tagName === "select" && !this.value) this.value = child.value; },
      setAttribute(name, value) { this[name] = value; },
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
    coder_profiles: [{ id: "default", requests_file_writes: true, requests_command_execution: true, can_write: true, can_execute_commands: false, requires_backend_approval: true }],
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
    assert.equal(state.get("#coder-profile").value, "default");
    assert.match(state.get("#coder-profile-capabilities").textContent, /Requests: file writes, command execution/);
    assert.match(state.get("#coder-profile-capabilities").textContent, /Desktop supports approved file diffs only/);
    state.setProject(undefined);
    state.ui.reset();
    assert.equal(state.get("#coder-panel").hidden, true);
    assert.equal(state.get("#coder-changes").children.length, 0);
    assert.equal(state.get("#coder-check").disabled, true);
    assert.equal(state.get("#coder-profile").children.length, 0);
    assert.equal(state.get("#coder-profile-capabilities").textContent, "");
  } finally { state.restore(); }
});

test("late diff from earlier staged source cannot replace newer diff", async () => {
  const first = deferred();
  const second = deferred();
  const state = fixture(async (command, args) => {
    if (command === "coder_diff") return (args.staged ? second : first).promise;
    return ({ coder_profiles: [{ id: "default", requests_file_writes: false, requests_command_execution: false, can_write: false, can_execute_commands: false, requires_backend_approval: true }],
      coder_tree: { files: [], truncated: false }, coder_changes: [{ path: "code.rs", status: " M" }],
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

test("Coder only writes after explicit approval of the exact proposed diff", async () => {
  const calls = [];
  const state = fixture(async (command, args) => {
    calls.push([command, args]);
    if (command === "coder_profiles") return [{ id: "default", requests_file_writes: false, requests_command_execution: false, can_write: true, can_execute_commands: false, requires_backend_approval: true }];
    if (command === "coder_propose_edit") return { id: "proposal-1", project_id: "one", profile_id: "default", path: "src.rs", diff: "--- a/src.rs\\n+++ b/src.rs\\n-old\\n+new\\n" };
    if (command === "coder_approve_edit") return { path: "src.rs", checkpoint: ".aiagent/checkpoints/one.bak", digest: "hash" };
    if (command === "coder_reject_edit") return undefined;
    if (command === "coder_tree") return { files: [{ path: "src.rs", directory: false }], truncated: false };
    if (command === "coder_changes") return [];
    if (command === "coder_venv") return { present: false, interpreter: null, note: "Inspection only" };
    throw new Error(`unexpected command ${command}`);
  });
  try {
    state.ui.setMode("coder");
    await state.ui.refresh();
    state.get("#coder-file-path").value = "src.rs";
    state.get("#coder-prompt").value = "Change old to new";
    await state.get("#coder-composer").listeners.submit({ preventDefault() {} });
    assert.equal(calls.some(([command]) => command === "coder_approve_edit"), false);
    assert.equal(state.get("#coder-proposal").hidden, false);
    assert.equal(state.get("#coder-proposal-diff").textContent.includes("+new"), true);
    assert.equal(state.get("#coder-approve").disabled, false);
    await state.get("#coder-approve").click();
    assert.equal(calls.filter(([command]) => command === "coder_approve_edit").length, 1);
    assert.equal(state.get("#coder-proposal").hidden, true);
    assert.match(state.get("#coder-status").textContent, /checkpoint saved/);
  } finally { state.restore(); }
});

test("rejecting a Coder diff never calls apply", async () => {
  const calls = [];
  const state = fixture(async (command) => {
    calls.push(command);
    if (command === "coder_profiles") return [{ id: "default", requests_file_writes: false, requests_command_execution: false, can_write: true, can_execute_commands: false, requires_backend_approval: true }];
    if (command === "coder_propose_edit") return { id: "proposal-2", project_id: "one", profile_id: "default", path: "src.rs", diff: "+change" };
    if (command === "coder_reject_edit") return undefined;
    if (command === "coder_tree") return { files: [], truncated: false };
    if (command === "coder_changes") return [];
    if (command === "coder_venv") return { present: false, note: "Inspection only" };
  });
  try {
    state.ui.setMode("coder");
    await state.ui.refresh();
    state.get("#coder-file-path").value = "src.rs";
    state.get("#coder-prompt").value = "Change it";
    await state.get("#coder-composer").listeners.submit({ preventDefault() {} });
    await state.get("#coder-reject").click();
    assert.ok(calls.includes("coder_reject_edit"));
    assert.equal(calls.includes("coder_approve_edit"), false);
    assert.equal(state.get("#coder-proposal").hidden, true);
  } finally { state.restore(); }
});

test("non-Git project still shows tree and environment but disables Git check", async () => {
  const state = fixture(async (command) => {
    if (command === "coder_changes") throw new Error("no local .git");
    return ({ coder_profiles: [{ id: "default", requests_file_writes: false, requests_command_execution: false, can_write: false, can_execute_commands: false, requires_backend_approval: true }],
      coder_tree: { files: [{ path: "code.rs", directory: false }], truncated: false },
      coder_venv: { present: false, interpreter: null, note: "Inspection only" } })[command];
  });
  try {
    state.ui.setMode("coder");
    await state.ui.refresh();
    assert.equal(state.get("#coder-tree").children[0].textContent, "· code.rs");
    assert.match(state.get("#coder-changes").textContent, /Git changes unavailable/);
    assert.match(state.get("#coder-status").textContent, /Partial inspection/);
    assert.equal(state.get("#coder-check").disabled, true);
    assert.match(state.get("#coder-venv").textContent, /No .venv/);
  } finally { state.restore(); }
});
