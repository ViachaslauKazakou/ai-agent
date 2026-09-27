// The first desktop screen is an API smoke-test client.  It uses the same
// command envelope that a future chat UI will use, so this scaffold validates
// the Tauri boundary before adding streaming agent execution.

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

if (typeof window.reportFrontendError === "function") {
  window.reportFrontendError = (error) => {
    const status = document.querySelector("#status");
    if (status) status.textContent = `Frontend error: ${String(error)}`;
  };
}

const status = document.querySelector("#status");
const pathInput = document.querySelector("#project-path");
const apiVersion = document.querySelector("#api-version");
const capabilities = document.querySelector("#capabilities");
const models = document.querySelector("#models");
const providerInput = document.querySelector("#provider");
const modelInput = document.querySelector("#model");
const createSessionButton = document.querySelector("#create-session");
const session = document.querySelector("#session");
const messages = document.querySelector("#messages");
const trace = document.querySelector("#trace");
const promptInput = document.querySelector("#prompt");
const composer = document.querySelector("#composer");
const sendButton = document.querySelector("#send");
const thinking = document.querySelector("#thinking");
const attachments = document.querySelector("#attachments");
const usedTools = document.querySelector("#used-tools");
const quoteButton = document.querySelector("#quote");
const attachButton = document.querySelector("#attach");
const toolEvents = document.querySelector("#tool-events");
const settingsButton = document.querySelector("#settings");
const settingsDialog = document.querySelector("#settings-dialog");
const configEditor = document.querySelector("#config-json");
const providersEditor = document.querySelector("#providers-json");
const settingsStatus = document.querySelector("#settings-status");
const reloadSettingsButton = document.querySelector("#reload-settings");
const saveSettingsButton = document.querySelector("#save-settings");
let activeProject;
let activeSession;
let attachedFiles = [];
let usedToolNames = new Set();
let activityTimer;
let availableProviders = [];

function logStep(message, details = "") {
  const line = `[${new Date().toLocaleTimeString()}] ${message}${details ? `: ${details}` : ""}`;
  trace.textContent += `${line}\n`;
  trace.scrollTop = trace.scrollHeight;
  console.info(`[ai-agent] ${line}`);
}

function requestId() {
  return crypto.randomUUID();
}

// Rust serializes ApplicationEvent as a tagged object containing its own
// `payload`. Keeping the unwrap in one helper prevents each button handler
// from accidentally treating the outer event as the DTO itself.
function eventPayload(envelope, expectedType) {
  const event = envelope?.payload;
  if (event?.type === expectedType) {
    // Current Rust DTO shape: { type, payload }.
    return event.payload ?? {};
  }
  if (event?.[expectedType] !== undefined) {
    return event[expectedType];
  }
  // Keep the smoke-test client compatible with an older running backend that
  // returned the inner DTO directly instead of a tagged enum event.
  if (event && typeof event === "object" && !event.type) return event;
  throw new Error(`unexpected application event: ${event?.type || "missing"}`);
}

function providersFrom(envelope) {
  const payload = eventPayload(envelope, "models_listed");
  const providers = Array.isArray(payload)
    ? payload
    : payload?.providers || envelope?.providers || [];
  if (!Array.isArray(providers)) {
    throw new Error("models_listed event does not contain a providers array");
  }
  return providers;
}

// Renders a bounded, keyboard-accessible model list. Selecting a model only
// changes the pending session value; the backend receives it when the user
// explicitly creates the session.
function renderModels(providerList) {
  availableProviders = providerList;
  const selectedProvider = providerInput.value;
  providerInput.replaceChildren();
  for (const provider of providerList) {
    const option = document.createElement("option");
    option.value = provider.name;
    option.textContent = `${provider.name} (${provider.kind})`;
    providerInput.append(option);
  }
  providerInput.disabled = providerList.length === 0;
  if (providerList.length) {
    providerInput.value = providerList.some((provider) => provider.name === selectedProvider)
      ? selectedProvider
      : providerList[0].name;
  }
  models.replaceChildren();
  const activeProvider = providerList.find((provider) => provider.name === providerInput.value);
  const entries = (activeProvider ? [activeProvider] : providerList).flatMap((provider) =>
    provider.models.map((model) => ({ provider: provider.name, model })),
  );
  if (!entries.length) {
    const empty = document.createElement("span");
    empty.className = "empty-state";
    empty.textContent = "No models available";
    models.append(empty);
    return;
  }
  for (const entry of entries) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "model-item";
    button.dataset.model = entry.model;
    button.dataset.provider = entry.provider;
    button.title = `${entry.provider} / ${entry.model}`;
    button.innerHTML = `<span>${entry.model}</span><small>${entry.provider}</small>`;
    button.addEventListener("click", () => {
      modelInput.value = entry.model;
      providerInput.value = entry.provider;
      modelInput.dataset.provider = entry.provider;
      for (const item of models.querySelectorAll(".model-item")) item.classList.remove("selected");
      button.classList.add("selected");
      status.textContent = `Model selected: ${entry.model}`;
      logStep("model selected", `${entry.provider} / ${entry.model}`);
    });
    models.append(button);
  }
}

function selectProvider(providerName) {
  providerInput.value = providerName;
  modelInput.dataset.provider = providerName;
  const provider = availableProviders.find((item) => item.name === providerName);
  const firstModel = provider?.models?.[0];
  modelInput.value = firstModel || "";
  renderModels(availableProviders);
  status.textContent = `Provider selected: ${providerName}`;
  logStep("provider selected", providerName);
}

function showError(error) {
  status.textContent = "Service error";
  logStep("ERROR", String(error));
  const item = document.createElement("article");
  item.className = "message error";
  item.innerHTML = `<span class="message-label">ERROR</span><p></p>`;
  item.querySelector("p").textContent = String(error);
  messages.append(item);
}

function setThinking(value) {
  thinking.hidden = !value;
  if (value) status.textContent = "Agent is thinking…";
}

function renderComposerAttachments() {
  attachments.replaceChildren();
  attachments.hidden = attachedFiles.length === 0;
  for (const file of attachedFiles) {
    const chip = document.createElement("span");
    chip.className = "attachment-chip";
    chip.textContent = file.name;
    attachments.append(chip);
  }
}

function renderUsedTools() {
  usedTools.hidden = usedToolNames.size === 0;
  usedTools.textContent = usedToolNames.size ? `Tools: ${[...usedToolNames].join(", ")}` : "";
}

// Polls only the redacted current tool name while the request is running.
// Tool arguments and results never cross this UI boundary.
function showToolActivity(tool) {
  if (!tool) return;
  usedToolNames.add(tool);
  renderUsedTools();
  const item = document.createElement("div");
  item.className = "tool-event";
  item.textContent = `${new Date().toLocaleTimeString()}  ${tool}`;
  if (toolEvents.querySelector(".empty-state")) toolEvents.replaceChildren();
  toolEvents.append(item);
  while (toolEvents.children.length > 12) toolEvents.firstElementChild.remove();
  toolEvents.scrollTop = toolEvents.scrollHeight;
}

function startActivityPolling() {
  activityTimer = window.setInterval(async () => {
    try {
      showToolActivity(await invoke("tool_activity"));
    } catch (error) {
      logStep("activity polling failed", String(error));
    }
  }, 250);
}

function stopActivityPolling() {
  if (activityTimer) window.clearInterval(activityTimer);
  activityTimer = undefined;
}

async function loadSettings() {
  if (!activeProject) {
    settingsStatus.textContent = "Open a project first";
    return;
  }
  settingsStatus.textContent = "Loading…";
  try {
    const documents = await invoke("read_settings", { projectId: activeProject.id });
    configEditor.value = documents.config_json;
    providersEditor.value = documents.providers_json;
    settingsStatus.textContent = "Loaded";
  } catch (error) {
    settingsStatus.textContent = "Load failed";
    showError(error);
  }
}

settingsButton.addEventListener("click", async () => {
  if (!activeProject) {
    status.textContent = "Open a project before editing settings";
    return;
  }
  settingsDialog.showModal();
  await loadSettings();
});
reloadSettingsButton.addEventListener("click", loadSettings);
saveSettingsButton.addEventListener("click", async () => {
  if (!activeProject) return;
  settingsStatus.textContent = "Saving…";
  saveSettingsButton.disabled = true;
  try {
    await invoke("write_settings", {
      projectId: activeProject.id,
      configJson: configEditor.value,
      providersJson: providersEditor.value,
    });
    settingsStatus.textContent = "Saved";
    status.textContent = "Configuration saved";
    settingsDialog.close();
    const modelEnvelope = await refreshModels(activeProject.id);
    renderModels(providersFrom(modelEnvelope));
  } catch (error) {
    settingsStatus.textContent = "Save failed";
    showError(error);
  } finally {
    saveSettingsButton.disabled = false;
  }
});

async function execute(command) {
  const id = requestId();
  logStep("execute_command", `${command.type} (${id})`);
  try {
    const result = await invoke("execute_command", { requestId: id, command });
    logStep("command completed", `${command.type}, sequence ${result.sequence}`);
    return result;
  } catch (error) {
    logStep("command failed", `${command.type}: ${String(error)}`);
    throw error;
  }
}

async function refreshModels(projectId) {
  const id = requestId();
  logStep("refresh_models", `${projectId} (${id})`);
  try {
    const result = await invoke("refresh_models", { requestId: id, projectId });
    logStep("models refreshed", `sequence ${result.sequence}`);
    return result;
  } catch (error) {
    logStep("model refresh failed", String(error));
    throw error;
  }
}

async function loadCapabilities() {
  status.textContent = "Connecting…";
  logStep("startup", "loading capabilities");
  try {
    const envelope = await execute({ type: "get_capabilities" });
    const value = eventPayload(envelope, "capabilities");
    apiVersion.textContent = `v${value.api_version}`;
    capabilities.textContent = [
      value.streaming ? "streaming" : "request/response",
      value.cancellation ? "cancel" : "no cancel",
      value.confirmations ? "confirmations" : "read-only",
    ].join(" · ");
    status.textContent = "Service ready";
    const modelEnvelope = await execute({ type: "list_models" });
    const providerList = providersFrom(modelEnvelope);
    renderModels(providerList);
    logStep("startup complete", `${providerList.length} provider(s) in registry`);
  } catch (error) {
    showError(error);
  }
}

document.querySelector("#open-project").addEventListener("click", async () => {
  status.textContent = "Opening project…";
  logStep("open project", pathInput.value || "empty path");
  try {
    const envelope = await execute({
      type: "open_project",
      payload: { path: pathInput.value },
    });
    const project = eventPayload(envelope, "project_opened");
    activeProject = project;
    logStep("project opened", `${project.id}: ${project.path}`);
    createSessionButton.disabled = false;
    status.textContent = "Project opened; refreshing models…";
    const modelEnvelope = await refreshModels(activeProject.id);
    const providerList = providersFrom(modelEnvelope);
    renderModels(providerList);
    const selectedProvider = providerList.find((provider) => provider.models.includes(modelInput.value));
    if (selectedProvider) selectProvider(selectedProvider.name);
    status.textContent = `Open: ${project.id}`;
    const item = document.createElement("article");
    item.className = "message assistant";
    item.innerHTML = `<span class="message-label">PROJECT</span><p></p>`;
    item.querySelector("p").textContent = `Registered ${project.path}`;
    messages.append(item);
  } catch (error) {
    showError(error);
  }
});

document.querySelector("#choose-project").addEventListener("click", async () => {
  logStep("choose project", "opening native directory picker");
  try {
    const selected = await open({ directory: true, multiple: false, title: "Choose project directory" });
    if (typeof selected === "string") {
      pathInput.value = selected;
      logStep("project selected", selected);
      status.textContent = "Project selected";
    } else {
      logStep("choose project cancelled");
    }
  } catch (error) {
    showError(error);
  }
});

createSessionButton.addEventListener("click", async () => {
  if (!activeProject) return;
  try {
    const envelope = await execute({
      type: "create_session",
      payload: { project_id: activeProject.id, provider: modelInput.dataset.provider || "litellm", model: modelInput.value },
    });
    const created = eventPayload(envelope, "session_created");
    activeSession = created;
    session.textContent = `${created.model} · ${created.id.slice(0, 8)}`;
    promptInput.disabled = false;
    sendButton.disabled = false;
    status.textContent = "Session ready";
  } catch (error) {
    showError(error);
  }
});

providerInput.addEventListener("change", () => selectProvider(providerInput.value));

composer.addEventListener("submit", async (event) => {
  event.preventDefault();
  const prompt = promptInput.value.trim();
  if (!activeSession || !prompt) return;
  promptInput.disabled = true;
  sendButton.disabled = true;
  setThinking(true);
  usedToolNames.clear();
  renderUsedTools();
  toolEvents.replaceChildren();
  startActivityPolling();
  logStep("send message", `${activeSession.id}: ${prompt}`);
  try {
    const request = requestId();
    const envelope = await invoke("send_message", {
      requestId: request,
      sessionId: activeSession.id,
      prompt,
    });
    const response = eventPayload(envelope, "assistant_message");
    const item = document.createElement("article");
    item.className = "message assistant";
    item.innerHTML = `<span class="message-label">ASSISTANT</span><p></p>`;
    item.querySelector("p").textContent = response.content;
    messages.append(item);
    messages.scrollTop = messages.scrollHeight;
    activeSession.message_count += 2;
    status.textContent = `Completed · ${response.tool_rounds} tool round(s)`;
    logStep("agent completed", `${response.tool_rounds} tool round(s)`);
    promptInput.value = "";
  } catch (error) {
    showError(error);
  } finally {
    stopActivityPolling();
    setThinking(false);
    promptInput.disabled = false;
    sendButton.disabled = false;
    promptInput.focus();
  }
});

promptInput.addEventListener("input", () => {
  promptInput.style.height = "auto";
  promptInput.style.height = `${Math.min(promptInput.scrollHeight, 180)}px`;
});

promptInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    composer.requestSubmit();
  }
});

quoteButton.addEventListener("click", () => {
  const selection = window.getSelection()?.toString().trim();
  if (!selection) {
    status.textContent = "Select text in the conversation first";
    return;
  }
  promptInput.value += `${promptInput.value ? "\n\n" : ""}> ${selection.split("\n").join("\n> ")}\n`;
  promptInput.dispatchEvent(new Event("input"));
  promptInput.focus();
});

attachButton.addEventListener("click", async () => {
  try {
    const selected = await open({ multiple: true, directory: false, title: "Attach files" });
    if (Array.isArray(selected)) attachedFiles = selected.map((path) => ({ name: path.split(/[\\/]/).pop(), path }));
    else if (typeof selected === "string") attachedFiles = [{ name: selected.split(/[\\/]/).pop(), path: selected }];
    renderComposerAttachments();
    logStep("files attached", `${attachedFiles.length} file(s)`);
  } catch (error) {
    showError(error);
  }
});

document.querySelector("#refresh").addEventListener("click", loadCapabilities);
logStep("frontend ready");
loadCapabilities();
