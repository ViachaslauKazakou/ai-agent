# Desktop launcher

The desktop application starts on a project and mode launcher instead of rendering an inactive chat workspace.

## Navigation model

`frontend/state.js` owns pure route and project-opening transitions. The initial route is `launcher`. Opening begins without changing routes, and `workspace` is entered only after Rust returns a valid `project_opened` event. A failed request keeps the launcher active and displays a recoverable error.

`Switch project` returns to the launcher and clears the active runtime project, session, provider list, and composer availability. It does not remove persisted recent projects or close backend project metadata; those records remain available for fast selection.

## Modes

- `Chat-bot` activates the implemented project selection flow.
- `Assistant`, `AI Tutor`, and `Coder` communicate their planned status without pretending that backend capabilities exist.
- `RAG Knowledge` is disabled and explicitly labelled as a future indexing feature.

Future mode stages should add backend capabilities before making their controls actionable.

## Recent projects

The launcher requests `get_startup_state` after capabilities are loaded. Available entries can open their canonical path. Missing entries remain visible but disabled, with an explanation that the path is unavailable. The suggested project is visually highlighted but never opened without user action.

When a project has a previously selected Desktop session, opening it
reactivates the same session UUID, provider, and model. The composer becomes
available immediately. Conversation content is not restored in this stage;
durable multi-session history remains a separate storage phase.

The browser layer receives only the metadata documented in `docs/desktop-startup.md`; it never reads `~/.ai-agent/state.json` directly.

## Layout and accessibility

Desktop layout is constrained to the window viewport. The workspace sidebar and chat messages have independent scroll containers. At narrow widths the page switches to a document flow and stacks the workspace to remain usable.

Route changes update visibility through native `hidden` attributes. Project controls have labels, startup and opening messages use live regions, unavailable projects are disabled, and returning to the launcher moves focus to project selection.

## Automated verification

The pure state model uses the Node built-in test runner, so frontend tests do not require a running Tauri process:

```bash
npm --prefix frontend run test
npm --prefix frontend run check
npm --prefix frontend run build
```

## Manual smoke test

1. Start with `make desktop` and confirm the launcher appears before the workspace.
2. Confirm all four mode cards are visible and unfinished modes show a planned-feature message.
3. Open a recent project and verify that workspace, models, sessions, settings, and chat remain functional.
4. Use `Switch project` and verify that the session/composer is reset.
5. Try an invalid path and confirm the launcher remains visible with an error.
6. Restart Desktop and confirm the opened project appears in recent projects.
7. Temporarily move that project and confirm its recent entry is visible but disabled.
8. Resize to `760x520` and confirm project/sidebar and chat content can be scrolled independently.
