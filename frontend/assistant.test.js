import test from "node:test";
import assert from "node:assert/strict";
import { actionAvailability, buildAssistantPrompt } from "./assistant.js";

test("actions fail closed without backend permissions", () => {
  assert.equal(actionAvailability(undefined, "mail").available, false);
  assert.equal(actionAvailability({ actions: [{ name: "mail", available: false, reason: "Tool disabled" }] }, "mail").reason, "Tool disabled");
  assert.equal(actionAvailability({ actions: [{ name: "calendar", available: true }] }, "mail").available, false);
});

test("shortcuts use read-only tools and never embed local document contents", () => {
  assert.match(buildAssistantPrompt("calendar"), /list_calendar_events/);
  assert.match(buildAssistantPrompt("mail"), /unread_only=true/);
  assert.match(buildAssistantPrompt("web_search", "  example  "), /example$/);
  assert.throws(() => buildAssistantPrompt("web_search", "  "));
  assert.match(buildAssistantPrompt("local_document"), /selected project document/);
  assert.throws(() => buildAssistantPrompt("cloud_document"));
});
