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
const modelInput = document.querySelector("#model");
const createSessionButton = document.querySelector("#create-session");
const session = document.querySelector("#session");
const messages = document.querySelector("#messages");
const trace = document.querySelector("#trace");
const promptInput = document.querySelector("#prompt");
const composer = document.querySelector("#composer");
const sendButton = document.querySelector("#send");
let activeProject;
let activeSession;

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
  models.replaceChildren();
  const entries = providerList.flatMap((provider) =>
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
    button.title = `${entry.provider} / ${entry.model}`;
    button.innerHTML = `<span>${entry.model}</span><small>${entry.provider}</small>`;
    button.addEventListener("click", () => {
      modelInput.value = entry.model;
      for (const item of models.querySelectorAll(".model-item")) item.classList.remove("selected");
      button.classList.add("selected");
      status.textContent = `Model selected: ${entry.model}`;
      logStep("model selected", `${entry.provider} / ${entry.model}`);
    });
    models.append(button);
  }
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
      payload: { project_id: activeProject.id, model: modelInput.value },
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

composer.addEventListener("submit", async (event) => {
  event.preventDefault();
  const prompt = promptInput.value.trim();
  if (!activeSession || !prompt) return;
  promptInput.disabled = true;
  sendButton.disabled = true;
  status.textContent = "Agent is working…";
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
    promptInput.disabled = false;
    sendButton.disabled = false;
    promptInput.focus();
  }
});

document.querySelector("#refresh").addEventListener("click", loadCapabilities);
logStep("frontend ready");
loadCapabilities();
