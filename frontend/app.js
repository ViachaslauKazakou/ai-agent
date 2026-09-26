// The first desktop screen is an API smoke-test client.  It uses the same
// command envelope that a future chat UI will use, so this scaffold validates
// the Tauri boundary before adding streaming agent execution.

import { invoke } from "@tauri-apps/api/core";

const status = document.querySelector("#status");
const pathInput = document.querySelector("#project-path");
const apiVersion = document.querySelector("#api-version");
const capabilities = document.querySelector("#capabilities");
const messages = document.querySelector("#messages");

function requestId() {
  return crypto.randomUUID();
}

function showError(error) {
  status.textContent = "Service error";
  const item = document.createElement("article");
  item.className = "message error";
  item.innerHTML = `<span class="message-label">ERROR</span><p></p>`;
  item.querySelector("p").textContent = String(error);
  messages.append(item);
}

async function execute(command) {
  return invoke("execute_command", {
    requestId: requestId(),
    command,
  });
}

async function loadCapabilities() {
  try {
    const envelope = await execute({ type: "get_capabilities" });
    const value = envelope.payload;
    apiVersion.textContent = `v${value.api_version}`;
    capabilities.textContent = [
      value.streaming ? "streaming" : "request/response",
      value.cancellation ? "cancel" : "no cancel",
      value.confirmations ? "confirmations" : "read-only",
    ].join(" · ");
    status.textContent = "Service ready";
  } catch (error) {
    showError(error);
  }
}

document.querySelector("#open-project").addEventListener("click", async () => {
  try {
    const envelope = await execute({
      type: "open_project",
      payload: { path: pathInput.value },
    });
    const project = envelope.payload;
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

document.querySelector("#refresh").addEventListener("click", loadCapabilities);
loadCapabilities();
