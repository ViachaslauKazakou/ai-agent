export function createCoderUI({ invoke, getProject }) {
  const panel = document.querySelector("#coder-panel");
  const status = document.querySelector("#coder-status");
  const tree = document.querySelector("#coder-tree");
  const changes = document.querySelector("#coder-changes");
  const diff = document.querySelector("#coder-diff");
  const venv = document.querySelector("#coder-venv");
  const profileSelect = document.querySelector("#coder-profile");
  const profileCapabilities = document.querySelector("#coder-profile-capabilities");
  const composer = document.querySelector("#coder-composer");
  const pathInput = document.querySelector("#coder-file-path");
  const promptInput = document.querySelector("#coder-prompt");
  const proposeButton = document.querySelector("#coder-propose");
  const proposalPanel = document.querySelector("#coder-proposal");
  const proposalPath = document.querySelector("#coder-proposal-path");
  const proposalDiff = document.querySelector("#coder-proposal-diff");
  const approveButton = document.querySelector("#coder-approve");
  const rejectButton = document.querySelector("#coder-reject");
  const staged = document.querySelector("#coder-staged");
  const refreshButton = document.querySelector("#coder-refresh");
  const checkButton = document.querySelector("#coder-check");
  let mode = "chatbot";
  let revision = 0;
  let selected;
  let diffRevision = 0;
  let busy = false;
  let gitAvailable = false;
  let selectedProfile = "";
  let profilesSnapshot = [];
  let proposal;
  let proposalRevision = 0;
  function showProfile(profile) {
    profileCapabilities.textContent = profile
      ? `Requests: ${profile.requests_file_writes ? "file writes" : "no file writes"}, ${profile.requests_command_execution ? "command execution" : "no command execution"}. Desktop supports approved file diffs only; commands remain disabled.`
      : "No profiles available.";
  }
  function render() {
    panel.hidden = mode !== "coder" || !getProject();
    refreshButton.disabled = busy;
    checkButton.disabled = busy || !getProject() || !gitAvailable;
    proposeButton.disabled = busy || !getProject() || !selectedProfile || Boolean(proposal);
    approveButton.disabled = busy || !proposal;
    rejectButton.disabled = busy || !proposal;
  }
  function reset(previousProjectId) {
    const previousProposal = proposal;
    if (previousProposal) {
      void invoke("coder_reject_edit", { proposalId: previousProposal.id }).catch(() => {});
    }
    if (previousProjectId) {
      void invoke("coder_clear_proposals", { projectId: previousProjectId }).catch(() => {});
    }
    revision++;
    diffRevision++;
    busy = false;
    gitAvailable = false;
    selected = undefined;
    selectedProfile = "";
    profilesSnapshot = [];
    proposalRevision++;
    proposal = undefined;
    proposalPanel.hidden = true;
    proposalPath.textContent = "";
    proposalDiff.textContent = "";
    promptInput.value = "";
    pathInput.value = "";
    profileSelect.replaceChildren();
    profileCapabilities.textContent = "";
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
      const [profiles, files, changed, environment] = await Promise.allSettled([
        invoke("coder_profiles", { projectId: project.id }),
        invoke("coder_tree", { projectId: project.id }),
        invoke("coder_changes", { projectId: project.id }),
        invoke("coder_venv", { projectId: project.id }),
      ]);
      if (current !== revision || getProject()?.id !== project.id) return;
      tree.replaceChildren();
      changes.replaceChildren();
      profileSelect.replaceChildren();
      if (profiles.status === "fulfilled") {
        profilesSnapshot = profiles.value;
        for (const profile of profilesSnapshot) {
          const option = document.createElement("option");
          option.value = profile.id;
          option.textContent = profile.id;
          profileSelect.append(option);
        }
        selectedProfile = profilesSnapshot.some((profile) => profile.id === selectedProfile)
          ? selectedProfile
          : profilesSnapshot[0]?.id ?? "";
        profileSelect.value = selectedProfile;
        showProfile(profilesSnapshot.find((profile) => profile.id === selectedProfile));
      } else {
        profilesSnapshot = [];
        selectedProfile = "";
        proposalRevision++;
        proposal = undefined;
        proposalPanel.hidden = true;
        const option = document.createElement("option");
        option.value = "";
        option.textContent = "Profiles unavailable";
        profileSelect.append(option);
        profileCapabilities.textContent = `Profile capabilities unavailable: ${String(profiles.reason)}`;
      }
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
  async function proposeEdit(event) {
    event.preventDefault();
    const project = getProject();
    const prompt = promptInput.value.trim();
    const path = pathInput.value.trim();
    if (!project || !selectedProfile || !path || !prompt || busy || proposal) return;
    const currentProject = project.id;
    const currentProfile = selectedProfile;
    const request = ++proposalRevision;
    busy = true;
    render();
    status.textContent = "Preparing a diff; project files are not changed…";
    try {
      const next = await invoke("coder_propose_edit", {
        projectId: currentProject,
        profileId: currentProfile,
        path,
        prompt,
      });
      if (request !== proposalRevision || getProject()?.id !== currentProject || selectedProfile !== currentProfile) {
        void invoke("coder_reject_edit", { proposalId: next.id }).catch(() => {});
        return;
      }
      proposal = next;
      proposalPath.textContent = next.path;
      proposalDiff.textContent = next.diff;
      proposalPanel.hidden = false;
      status.textContent = "Review this exact diff. It will not be applied until you approve it.";
    } catch (error) {
      if (request === proposalRevision && getProject()?.id === currentProject) {
        status.textContent = `Could not prepare diff: ${String(error)}`;
      }
    } finally {
      if (request === proposalRevision) { busy = false; render(); }
    }
  }
  async function resolveProposal(approve) {
    const project = getProject();
    const current = proposal;
    if (!project || !current || busy) return;
    const request = proposalRevision;
    busy = true;
    render();
    let outcomeMessage;
    try {
      if (approve) {
        const result = await invoke("coder_approve_edit", {
          projectId: project.id,
          profileId: current.profile_id,
          proposalId: current.id,
        });
        if (request !== proposalRevision || getProject()?.id !== current.project_id) return;
        proposalRevision++;
        proposal = undefined;
        proposalPanel.hidden = true;
        proposalPath.textContent = "";
        proposalDiff.textContent = "";
        outcomeMessage = `Applied ${result.path}; checkpoint saved at ${result.checkpoint}.`;
      } else {
        await invoke("coder_reject_edit", { proposalId: current.id });
        if (request !== proposalRevision || getProject()?.id !== current.project_id) return;
        proposalRevision++;
        proposal = undefined;
        proposalPanel.hidden = true;
        proposalPath.textContent = "";
        proposalDiff.textContent = "";
        outcomeMessage = "Diff rejected; no project file was changed.";
      }
      await refresh();
      if (proposalRevision === request + 1 && getProject()?.id === current.project_id) {
        status.textContent = outcomeMessage;
      }
    } catch (error) {
      if (request === proposalRevision && getProject()?.id === current.project_id) {
        status.textContent = `Could not ${approve ? "apply" : "reject"} diff: ${String(error)}`;
      }
    } finally {
      if (request === proposalRevision) { busy = false; render(); }
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
  composer.addEventListener("submit", proposeEdit);
  approveButton.addEventListener("click", () => resolveProposal(true));
  rejectButton.addEventListener("click", () => resolveProposal(false));
  profileSelect.addEventListener("change", async () => {
    if (proposal) {
      const previous = proposal;
      proposal = undefined;
      proposalRevision++;
      proposalPanel.hidden = true;
      try { await invoke("coder_reject_edit", { proposalId: previous.id }); } catch {}
    }
    selectedProfile = profileSelect.value;
    showProfile(profilesSnapshot.find((profile) => profile.id === selectedProfile));
    render();
  });
  checkButton.addEventListener("click", check);
  staged.addEventListener("change", () => { if (selected) loadDiff(selected); });
  render();
  return { reset, render, refresh, setMode };
}
