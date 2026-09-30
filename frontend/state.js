/** Routes rendered by the desktop shell. */
export const ROUTES = Object.freeze({
  LAUNCHER: "launcher",
  WORKSPACE: "workspace",
});

/** Only modes with dedicated backend paths may become active workspaces. */
export function canActivateMode(mode) {
  return mode === "chatbot" || mode === "assistant" || mode === "coder";
}

/** Creates isolated UI state for one desktop window. */
export function createAppState() {
  return {
    route: ROUTES.LAUNCHER,
    projectStatus: "idle",
    activeProject: null,
    startup: {
      suggestedProjectId: null,
      recentProjects: [],
    },
  };
}

/** Normalizes the secret-free startup DTO returned by Rust. */
export function applyStartupState(state, startup = {}) {
  return {
    ...state,
    startup: {
      suggestedProjectId: startup.suggested_project_id ?? null,
      recentProjects: Array.isArray(startup.recent_projects) ? startup.recent_projects : [],
    },
  };
}

/** Marks project opening without navigating away from the launcher. */
export function beginProjectOpen(state) {
  return { ...state, projectStatus: "opening" };
}

/** Enters the workspace only after the backend accepted the project. */
export function completeProjectOpen(state, project) {
  if (!project?.id || !project?.path) throw new Error("opened project is incomplete");
  return {
    ...state,
    route: ROUTES.WORKSPACE,
    projectStatus: "open",
    activeProject: project,
  };
}

/** Keeps launcher context intact after a recoverable opening error. */
export function failProjectOpen(state) {
  return { ...state, route: ROUTES.LAUNCHER, projectStatus: "error", activeProject: null };
}

/** Returns to project selection and clears project-scoped runtime selection. */
export function returnToLauncher(state) {
  return { ...state, route: ROUTES.LAUNCHER, projectStatus: "idle", activeProject: null };
}
