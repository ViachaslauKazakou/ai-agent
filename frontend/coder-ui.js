export function createCoderUI({ invoke, getProject }) {
  const panel = document.querySelector("#coder-panel");
  const status = document.querySelector("#coder-status");
  const tree = document.querySelector("#coder-tree");
  const changes = document.querySelector("#coder-changes");
  const diff = document.querySelector("#coder-diff");
  const venv = document.querySelector("#coder-venv");
  const staged = document.querySelector("#coder-staged");
  const refreshButton = document.querySelector("#coder-refresh");
  const checkButton = document.querySelector("#coder-check");
  let mode = "chatbot";
  let revision = 0;
  let selected;
  let diffRevision = 0;
  let busy = false;
  let gitAvailable = false;
  function render() {
    panel.hidden = mode !== "coder" || !getProject();
    refreshButton.disabled = busy;
    checkButton.disabled = busy || !getProject() || !gitAvailable;
  }
  function reset() {
    revision++;
    diffRevision++;
    busy = false;
    gitAvailable = false;
    selected = undefined;
    tree.replaceChildren();
    changes.replaceChildren();
    diff.textContent = "Select a changed file to view its diff.";
    venv.textContent = "";
    status.textContent = "";
    render();
  }
  function setMode(next) { mode = next; render(); }
  async function loadDiff(path) {
    const project = getProject();
    if (!project) return;
    selected = path;
    const current = revision;
    const request = ++diffRevision;
    diff.textContent = "Loading diff…";
    try {
      const result = await invoke("coder_diff", { projectId: project.id, path, staged: staged.value === "true" });
      if (current === revision && request === diffRevision && getProject()?.id === project.id && selected === path) diff.textContent = result.content || "No tracked changes in this source (untracked files have no diff).";
    } catch (error) {
      if (current === revision && request === diffRevision && getProject()?.id === project.id && selected === path) diff.textContent = `Diff unavailable: ${String(error)}`;
    }
  }
  async function refresh() {
    const project = getProject();
    if (!project) return;
    const current = ++revision;
    diffRevision++;
    busy = true;
    render();
    status.textContent = "Loading project inspection…";
    try {
      const [files, changed, environment] = await Promise.allSettled([
        invoke("coder_tree", { projectId: project.id }),
        invoke("coder_changes", { projectId: project.id }),
        invoke("coder_venv", { projectId: project.id }),
      ]);
      if (current !== revision || getProject()?.id !== project.id) return;
      tree.replaceChildren();
      changes.replaceChildren();
      gitAvailable = changed.status === "fulfilled";
      if (files.status === "fulfilled") {
        for (const file of files.value.files) {
          const line = document.createElement("div");
          line.textContent = `${file.directory ? "▸" : "·"} ${file.path}`;
          tree.append(line);
        }
        if (!files.value.files.length) tree.textContent = "No visible files";
      } else {
        tree.textContent = `Files unavailable: ${String(files.reason)}`;
      }
      if (gitAvailable) {
        for (const change of changed.value) {
          const button = document.createElement("button");
          button.type = "button";
          button.textContent = `${change.status}  ${change.path}`;
          button.addEventListener("click", () => loadDiff(change.path));
          changes.append(button);
        }
        if (!changed.value.length) changes.textContent = "No visible changes";
      } else {
        changes.textContent = `Git changes unavailable: ${String(changed.reason)}`;
      }
      venv.textContent = environment.status === "fulfilled"
        ? `${environment.value.present ? ".venv present" : "No .venv"}${environment.value.interpreter ? ` · ${environment.value.interpreter}` : ""}. ${environment.value.note}`
        : `Environment unavailable: ${String(environment.reason)}`;
      selected = undefined;
      diff.textContent = "Select a changed file to view its diff.";
      status.textContent = `${files.status === "fulfilled" && gitAvailable && environment.status === "fulfilled" ? "Ready" : "Partial inspection"} · ${files.status === "fulfilled" ? `${files.value.files.length} entries${files.value.truncated ? " (tree truncated)" : ""}` : "files unavailable"} · ${gitAvailable ? `${changed.value.length} changes` : "Git unavailable"}`;
    } finally {
      if (current === revision) { busy = false; render(); }
    }
  }
  async function check() {
    const project = getProject();
    if (!project || busy || !gitAvailable) return;
    const current = revision;
    busy = true;
    render();
    status.textContent = "Checking whitespace…";
    try {
      const result = await invoke("coder_check", { projectId: project.id });
      if (current === revision && getProject()?.id === project.id) status.textContent = `${result.name}: ${result.passed ? "passed" : "failed"} · ${result.details}`;
    } catch (error) {
      if (current === revision && getProject()?.id === project.id) status.textContent = `Check unavailable: ${String(error)}`;
    } finally { if (current === revision) { busy = false; render(); } }
  }
  refreshButton.addEventListener("click", refresh);
  checkButton.addEventListener("click", check);
  staged.addEventListener("change", () => { if (selected) loadDiff(selected); });
  render();
  return { reset, render, refresh, setMode };
}
