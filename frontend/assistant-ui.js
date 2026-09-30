import { actionAvailability, buildAssistantPrompt } from "./assistant.js";

export function createAssistantUI({ invoke, open, promptInput, getProject, getSession, isBusy }) {
  const panel = document.querySelector("#assistant-panel");
  const status = document.querySelector("#assistant-status");
  const select = document.querySelector("#assistant-preset");
  const dialog = document.querySelector("#assistant-dialog");
  const nameInput = document.querySelector("#assistant-prompt-name");
  const instructionInput = document.querySelector("#assistant-prompt-instruction");
  const feedback = document.querySelector("#assistant-prompt-status");
  let capabilities;
  let prompts = [];
  let mode = "chatbot";
  let documentPath;
  let loading = false;
  let revision = 0;
  let pending = false;
  let selectedPreset = "";

  function render() {
    panel.hidden = mode !== "assistant" || !getProject();
    for (const button of panel.querySelectorAll("[data-assistant-action]")) {
      const action = actionAvailability(capabilities, button.dataset.assistantAction);
      button.disabled = pending || loading || isBusy() || !getSession() || !action.available;
      button.title = !getSession() ? "Create a session first" : action.reason || "Read-only action";
    }
    select.replaceChildren(new Option("Choose a preset…", ""));
    for (const preset of prompts) select.add(new Option(preset.name, preset.name));
    if (!prompts.some((preset) => preset.name === selectedPreset)) selectedPreset = "";
    select.value = selectedPreset;
    select.disabled = pending || loading || isBusy() || !getSession() || !prompts.length;
    document.querySelector("#assistant-manage").disabled = pending || loading || isBusy() || !getProject();
    document.querySelector("#assistant-refresh").disabled = pending || loading || isBusy();
  }

  async function refresh() {
    const project = getProject();
    if (!project) return;
    const request = ++revision;
    capabilities = undefined;
    loading = true;
    render();
    status.textContent = "Loading permissions…";
    try {
      const [available, saved] = await Promise.all([
        invoke("assistant_capabilities", { projectId: project.id }),
        invoke("assistant_prompts", { projectId: project.id }),
      ]);
      if (getProject()?.id !== project.id || request !== revision) return;
      capabilities = available;
      prompts = saved;
      status.textContent = "Ready · actions follow project permissions";
    } catch (error) {
      if (getProject()?.id !== project.id || request !== revision) return;
      status.textContent = `Assistant unavailable: ${String(error)}`;
    } finally {
      if (request === revision) { loading = false; render(); }
    }
  }

  function reset() {
    ++revision;
    loading = false;
    pending = false;
    capabilities = undefined;
    prompts = [];
    documentPath = undefined;
    selectedPreset = "";
    if (dialog.open) dialog.close();
    render();
  }

  function setMode(next) {
    mode = next;
    documentPath = undefined;
    render();
  }

  async function action(name) {
    const available = actionAvailability(capabilities, name);
    if (pending || loading || isBusy() || !getSession() || !available.available) {
      status.textContent = available.reason || "Create a session first";
      return;
    }
    const project = getProject();
    const sessionId = getSession().id;
    const request = revision;
    pending = true;
    render();
    try {
      let details = "";
      let selectedPath;
      if (name === "web_search") {
        details = window.prompt("Search the web for:") || "";
        if (!details.trim()) return;
      }
      if (name === "local_document") {
        selectedPath = await open({ directory: false, multiple: false, title: "Select a project document" });
        if (!selectedPath || typeof selectedPath !== "string") return;
      }
      if (revision !== request || getProject()?.id !== project?.id || isBusy() || getSession()?.id !== sessionId) return;
      documentPath = selectedPath;
      promptInput.value = buildAssistantPrompt(name, details);
      promptInput.dispatchEvent(new Event("input"));
      status.textContent = selectedPath ? "Project document selected; content stays in Rust until Send" : "Request ready; review and press Send";
      promptInput.focus();
    } catch (error) {
      if (revision === request && getProject()?.id === project?.id) status.textContent = `Action failed: ${String(error)}`;
    } finally {
      if (request === revision) { pending = false; render(); }
    }
  }

  async function persist(next) {
    const project = getProject();
    if (!project) return;
    const request = revision;
    pending = true;
    render();
    try {
      await invoke("save_assistant_prompts", { projectId: project.id, prompts: next });
      if (getProject()?.id === project.id && request === revision) { prompts = next; render(); }
    } finally {
      if (request === revision) { pending = false; render(); }
    }
  }

  for (const button of panel.querySelectorAll("[data-assistant-action]")) button.addEventListener("click", () => action(button.dataset.assistantAction));
  document.querySelector("#assistant-refresh").addEventListener("click", refresh);
  document.querySelector("#assistant-manage").addEventListener("click", () => {
    nameInput.value = select.value;
    instructionInput.value = prompts.find((entry) => entry.name === select.value)?.instruction || "";
    feedback.textContent = "";
    dialog.showModal();
  });
  select.addEventListener("change", () => {
    selectedPreset = select.value;
    const preset = prompts.find((entry) => entry.name === select.value);
    if (!preset || !getSession() || isBusy()) return;
    promptInput.value = preset.instruction;
    documentPath = undefined;
    promptInput.dispatchEvent(new Event("input"));
    status.textContent = "Preset ready; review and press Send";
  });
  document.querySelector("#assistant-prompt-save").addEventListener("click", async () => {
    const name = nameInput.value.trim();
    const instruction = instructionInput.value.trim();
    if (!name || !instruction || !getProject()) { feedback.textContent = "Name and instruction are required"; return; }
    if (pending) return;
    const existing = selectedPreset;
    const next = prompts.filter((entry) => entry.name !== existing);
    next.push({ name, instruction });
    const project = getProject();
    const request = revision;
    try { await persist(next); if (request === revision && getProject()?.id === project.id) { selectedPreset = name; render(); feedback.textContent = "Saved"; } }
    catch (error) { if (request === revision && getProject()?.id === project.id) feedback.textContent = String(error); }
  });
  document.querySelector("#assistant-prompt-delete").addEventListener("click", async () => {
    if (pending) return;
    if (!selectedPreset || !getProject()) { feedback.textContent = "Select a preset first"; return; }
    const project = getProject();
    const request = revision;
    try { await persist(prompts.filter((entry) => entry.name !== selectedPreset)); if (request === revision && getProject()?.id === project.id) feedback.textContent = "Deleted"; }
    catch (error) { if (request === revision && getProject()?.id === project.id) feedback.textContent = String(error); }
  });
  render();
  return { render, refresh, reset, setMode, documentPath: () => documentPath, clearDocument: () => { documentPath = undefined; } };
}
