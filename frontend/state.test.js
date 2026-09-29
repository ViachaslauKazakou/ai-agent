import test from "node:test";
import assert from "node:assert/strict";

import {
  ROUTES,
  applyStartupState,
  beginProjectOpen,
  completeProjectOpen,
  createAppState,
  failProjectOpen,
  returnToLauncher,
} from "./state.js";

test("startup projects are normalized without changing the initial route", () => {
  const state = applyStartupState(createAppState(), {
    suggested_project_id: "project-a",
    recent_projects: [{ id: "project-a", path: "/tmp/a", available: true }],
  });

  assert.equal(state.route, ROUTES.LAUNCHER);
  assert.equal(state.startup.suggestedProjectId, "project-a");
  assert.equal(state.startup.recentProjects.length, 1);
});

test("workspace is entered only after project opening completes", () => {
  const opening = beginProjectOpen(createAppState());
  assert.equal(opening.route, ROUTES.LAUNCHER);
  assert.equal(opening.projectStatus, "opening");

  const opened = completeProjectOpen(opening, { id: "project-a", path: "/tmp/a" });
  assert.equal(opened.route, ROUTES.WORKSPACE);
  assert.equal(opened.activeProject.id, "project-a");
});

test("failed opening remains recoverable on launcher", () => {
  const failed = failProjectOpen(beginProjectOpen(createAppState()));

  assert.equal(failed.route, ROUTES.LAUNCHER);
  assert.equal(failed.projectStatus, "error");
  assert.equal(failed.activeProject, null);
});

test("returning to launcher clears project-scoped selection", () => {
  const opened = completeProjectOpen(createAppState(), { id: "project-a", path: "/tmp/a" });
  const returned = returnToLauncher(opened);

  assert.equal(returned.route, ROUTES.LAUNCHER);
  assert.equal(returned.activeProject, null);
});
