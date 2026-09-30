# Running via-agent

This guide explains how to configure and run both clients:

- `ai-agent` — the existing console client;
- `via-agent` — the Tauri desktop client.

Both clients use the same Rust agent core, project-local configuration, provider
registry, tools, sessions, and OS credential storage.

## Requirements

For the console client:

- Rust 1.89 or newer;
- Cargo;
- a reachable Ollama or LiteLLM/OpenAI-compatible endpoint.

For desktop development:

- Rust and Cargo;
- Node.js and npm;
- Tauri 2 CLI;
- the WebView development dependencies required by Tauri on the target OS.

Install the Tauri CLI once:

```bash
cargo install tauri-cli --version '^2'
```

## Project configuration

Configuration is loaded from the selected project's `.aiagent/` directory. It
is not required to export the same values as environment variables.

Initialize a project:

```bash
ai-agent --working-dir /path/to/project --init
```

The important files are:

```text
/path/to/project/.aiagent/config.json
/path/to/project/.aiagent/providers.json
/path/to/project/.aiagent/agents/default.toml
```

`config.json` contains the selected provider/model, permissions, enabled tools,
OAuth client settings, and runtime limits. `providers.json` contains provider
endpoints, API keys, models, and provider capability flags.

Example provider registry:

```json
{
  "providers": {
    "litellm": {
      "kind": "openai-compatible",
      "base_url": "http://localhost:4000/v1",
      "api_key": "your-key",
      "models": ["elite-gpt-5.6-luna"],
      "supports_reasoning_effort": false,
      "supports_reasoning_with_tools": false,
      "reasoning_effort_models": [],
      "reasoning_with_tools_models": []
    },
    "ollama": {
      "kind": "ollama",
      "base_url": "http://localhost:11434/v1",
      "api_key": null,
      "models": ["llama3.2"],
      "supports_reasoning_effort": false,
      "supports_reasoning_with_tools": false,
      "reasoning_effort_models": [],
      "reasoning_with_tools_models": []
    }
  }
}
```

Do not commit `.aiagent/config.json` or `.aiagent/providers.json` when they
contain credentials. Use OS keychain-backed OAuth tokens and local secret
management where possible.

## Console client

Run from the repository during development:

```bash
cargo run -- --working-dir /path/to/project
```

For interactive `ai-agent` without a directory argument or
`AI_AGENT_PROJECT_DIR`, subsequent launches offer available recent projects,
most recently opened first, plus the current directory. Use arrow keys and
Enter to select; Esc keeps the current directory. The first launch behaves as
before. Explicit `--project-dir`/`--working-dir`, one-shot prompts, `--init`,
OAuth login and scheduler skip the picker. After selecting a project, the CLI
still asks separately whether to load its legacy
`<working-dir>/.aiagent/session.json` (default: no). CLI history remains
separate from Desktop's `.aiagent/sessions/` histories.

Successful CLI opens record project metadata in `~/.ai-agent/state.json`,
shared with the Desktop launcher; no messages or credentials are recorded.
Missing recent directories are not offered. A corrupt/incompatible state file
is reported but preserved; explicit CLI paths continue to work.

Run a single prompt:

```bash
cargo run -- \
  --working-dir /path/to/project \
  "List the files in the project"
```

After installing the release binary:

```bash
make release
cargo install --path . --locked --force
ai-agent --working-dir /path/to/project
```

Provider/model can be selected explicitly:

```bash
ai-agent \
  --working-dir /path/to/project \
  --provider litellm \
  --model elite-gpt-5.6-luna \
  "Reply with OK only"
```

Useful REPL commands include `/models`, `/model`, `/provider`, `/tools`,
`/permissions`, `/config`, `/save`, `/load`, `/compact`, and `/quit`.

### Gmail OAuth

OAuth client settings are read from the selected project's config:

```bash
ai-agent \
  --working-dir /path/to/project \
  --gmail-login
```

The browser callback listens on `127.0.0.1:8765`. Refresh tokens are stored in
the OS credential store rather than in project JSON.

## Desktop client

### Development mode

Install frontend dependencies once:

```bash
cd frontend
npm install
cd ..
```

Start the desktop client:

```bash
make desktop
```

Development always uses `127.0.0.1:1420`, matching `tauri.conf.json`. Vite is
configured with a strict port and will stop with an explicit error instead of
silently moving to `1421`, which Tauri cannot load. If another previous dev
process still owns the port, stop that process with `Ctrl+C` before retrying.

Vite also loads its config natively. This prevents temporary bundled config
files under `frontend/node_modules/.vite-temp` from triggering Tauri's Rust
watcher during the initial binary link.

The Tauri window opens the launcher through the local Vite server. In the UI:

1. Select `Chat-bot` or `Assistant` on the mode screen. `AI Tutor` and `Coder`
   still show an explicit planned-feature notice.
2. Choose an available recent project, or click `Choose…` and select a project
   directory.
3. Click `Open project`. The chat workspace appears only after Rust validates
   the path and loads its configuration.
4. If a compatible indexed session or legacy `.aiagent/session.json` exists, its UUID, provider, model
   and history are restored in the backend; the UI displays a message count,
   not the previous conversation. Otherwise select a provider and model, then
    click `Create session`. Use `Recent sessions` and `Continue selected session`
    to switch back. `Create session` starts a separate history, including when a
    session is already active. Up to ten recent sessions are kept per project;
     creating the eleventh deletes the least recently updated. The dropdown
     shows a short first-prompt title and last-update time. `Delete selected
     session` asks for confirmation and removes only its Desktop history; the
     legacy CLI copy is unaffected. File attachments are not supported yet;
     there is no Attach button. A corrupt or
    incompatible history is kept intact and reported; repair it before creating
    new sessions.
5. Enter a prompt and click `Send`. Desktop does not stop at 20 tool rounds;
    it continues until the agent finishes or the configured `max_loop_seconds`
    timeout is reached (600 seconds by default). CLI requests still honor
    `max_tool_rounds`.
    Chat-bot can create and edit files in the selected project by default,
    without CLI `--allow-write`. Only tools in `enabled_tools` are available;
    the default set includes `write_file`, `create_file`, and `apply_patch`.
    Desktop applies patches without terminal confirmation, but does not enable
    Git commit/push or shell commands. Files outside the project (including via
    symlinks), secrets, `.git/`, and `.aiagent/` remain protected. File edits
    require a Git repository for checkpoints. If `working_dir` points to a
    different directory or `confirm_writes` is enabled, Desktop refuses file
    mutations rather than silently writing elsewhere or skipping a requested
    confirmation. The CLI retains its read-only default and interactive prompts.
6. Use `Switch project` to return to the launcher. The active project and
   session selection are cleared, while persisted recent projects remain.

### Assistant workspace

Choose Assistant before opening a project, then create or restore a session.
Calendar, Mail and Web Search buttons prepare editable requests; pressing Send
executes them through the read-only Rust agent tool registry. Availability comes
from the project's enabled tools and connector configuration. Refresh permissions
after changing settings. A configured client ID is not proof of a valid OAuth
login or network access: failed calls report an error rather than fabricating
results. See [mail setup](README.md#работа-с-почтой) for Gmail/Outlook OAuth;
Google Calendar requires `GOOGLE_CALENDAR_CLIENT_ID` and prior authorization,
or uses macOS Calendar access on macOS. Web Search defaults to DuckDuckGo; Tavily
requires `WEB_SEARCH_PROVIDER=tavily` and `WEB_SEARCH_API_KEY`.
Set `enabled_tools` explicitly in `.aiagent/config.json` to turn off individual
tools; automatic mail/calendar/macOS defaults apply only when the field is absent.
For example, removing `list_calendar_events` disables Calendar even on macOS.

Local document selects a file in the configured tool `working_dir` and prepares
a prompt. On Send, Rust calls `mcp_read_local_file` and sends the extracted text
to the selected LLM provider. The WebView receives neither raw tool output nor
the document text. The tool checks canonical paths (including symlinks), allows
only `toml`, `yml`, `yaml`, `txt`, `json`, `md`, `doc`, `docx`, `pdf`, limits input to
5 MB and Assistant prompt output to 30 KB; `pdf` requires `pdftotext`, `docx`
requires `unzip`, and `doc` requires `textutil`. Do not select a confidential
document unless you trust your configured LLM provider. Session history contains
the submitted document text; the global launcher state does not. Cloud document
access, recognition, conversion, one-click summary and feedback are disabled
placeholders, not working connectors.

Manage prompts stores up to 20 named project-local presets in versioned
`.aiagent/assistant-prompts.json`. Presets are editable instructions, not
independent subagents and never grant tools. An invalid or unknown-version
document is preserved and must be fixed explicitly. Assistant's allowlist
removes all file-write, process, Git mutation and mail-send tools, even when
Chat-bot in the same session permits writes; changing modes reloads the latest
checkpoint. The web search tool can contact external services; do not include
private data in search queries.

The RAG block is a disabled design placeholder; this stage does not index or
send project content to a retrieval service. Missing recent project paths are
shown as unavailable and cannot be opened from the list.

The gear button in the top-right opens editors for:

- `.aiagent/config.json`;
- `.aiagent/providers.json`.

The backend validates both JSON documents and writes them atomically. The
execution log is available at the bottom of the window. It is collapsed by
default and can be expanded when diagnosing provider or tool errors.

### Release desktop build

Build the console and desktop release artifacts together:

```bash
make release-all
```

Build only the desktop application:

```bash
make desktop-build
```

Artifacts are generated under:

```text
src-tauri/target/release/bundle/
```

Install a local `via-agent` launcher:

```bash
make desktop-install
via-agent
```

Open an existing project directly, or choose one from the launcher:

```bash
via-agent --working-dir /path/to/project
```

Invalid direct-open paths are reported to stderr and leave the launcher
available. Desktop histories are stored in `.aiagent/sessions/`; legacy
`.aiagent/session.json` is copied when first restored or before creating a
new Desktop session, and remains available to the CLI.

The console command remains:

```bash
ai-agent --working-dir /path/to/project
```

Remove the desktop launcher with:

```bash
make desktop-uninstall
```

## Provider troubleshooting

Test a LiteLLM/OpenAI-compatible endpoint directly:

```bash
curl -sS \
  -H "Authorization: Bearer $LITELLM_API_KEY" \
  -H "Content-Type: application/json" \
  http://localhost:4000/v1/chat/completions \
  -d '{"model":"elite-gpt-5.6-luna","messages":[{"role":"user","content":"Reply with OK only."}],"max_tokens":256}'
```

Test model discovery:

```bash
curl -sS -H "Authorization: Bearer $LITELLM_API_KEY" \
  http://localhost:4000/v1/models
```

For Ollama:

```bash
ollama serve
ollama list
curl http://localhost:11434/v1/models
```

If the UI shows an error, expand `Execution log` and inspect the terminal where
Tauri was started. The log includes request lifecycle events but excludes tool
arguments, tool results, API keys, and OAuth tokens.

## Verification commands

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
cd frontend && npm run check && npm run build
```
