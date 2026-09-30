# Desktop AI Tutor: bounded contract and threat model

## Status and source of requirements

This is the **contract-only** slice of stage 9. The launcher card is an inactive
placeholder: there is no lesson runner, provider request, audio capture, stream,
editor or progress store. Tutor capabilities are reported by the Rust application
service and are all disabled. Chat-bot/Assistant sessions and their send endpoints
are **not** Tutor sessions; selecting Tutor never starts a Chat-bot request.

The adjacent `learn-service/src/core/services/ai_tutor` (description, schemas,
router, session manager and lesson engine) was reviewed as product research.
Its repository declares MIT in `LICENSE` and `pyproject.toml`, but no source
code is copied here. Its FastAPI/WebSocket/JWT, PostgreSQL/SQLAlchemy,
subscription, quiz and RAG integrations do not establish permissions for a
local Rust/Tauri application. In particular, arbitrary message metadata,
model-generated actions, query-string credentials and unbounded audio reads
must not be imported as a Desktop security policy.

## Current Rust contract

`src/tutor.rs` defines transport-neutral, serializable `TutorLessonDto` (opaque
lesson ID, title, ordered questions), `TutorQuestionDto` (ID, title and text),
`TutorSessionDto` (opaque session/lesson/project IDs, status, phase, question
index), `TutorPhase`, `TutorSessionStatus` and `TutorCapabilitiesDto`. DTOs have
no filesystem path, URL, credentials, tools or free-form metadata. Lesson
validation is backend-owned: nonempty, byte-bounded strings; 1–20 questions,
unique nonempty question IDs and a 64 KiB serialized-document ceiling. The
current contract permits round-trip serialization and rejection of malformed
lesson descriptions but **does not accept, persist or execute lessons**.

`get_tutor_capabilities` requires an opened project with loaded configuration.
It returns `available=false`, `text=false`, `streaming=false`,
`cancellation=false`, `voice=false`, `editing=false`, `progress=false`, and
an explicit reason. These are Tutor-only flags, not the general
`ApplicationCapabilities.cancellation` flag (which describes a cooperative
handle that is not wired to Tutor). A future implementation must enable each
flag only after that operation has a tested, enforceable Rust boundary. The
Tauri adapter exposes the same service command via `execute_command`; it does
not expose a Tutor send or mutation endpoint.

## Planned lesson lifecycle (not implemented)

An explicitly created Tutor session would bind a validated immutable lesson
revision, canonical opened project identity and selected provider/model. It
would move `draft -> in_progress -> completed` or `abandoned`; only the backend
may advance `intro -> theory -> dialog -> quiz -> summary`, repeating
theory/dialog/quiz per question. Resume must validate both project and lesson
revision. Cancellation of one turn must leave a resumable checkpoint and must
not imply completion. Do not reuse Chat-bot's session UUID, history, tool
registry or automatic file-edit grant. Future grading/quiz answers are
untrusted data; the model cannot select a phase, issue tool calls, award scores
or mutate the lesson plan merely by returning JSON in text.

### Text streaming and cancellation (design only)

Use an ordered per-request stream of typed events with request/session IDs and
monotonic sequence numbers (start, bounded text delta, end/error/cancelled).
An adapter can use Tauri IPC channels, but transport cannot enforce permission
or substitute for bounded queues. Design limits: UTF-8 prompt <= 8 KiB,
delta <= 4 KiB, total response <= 64 KiB, at most one in-flight turn per
Tutor session, and a finite time/token budget on the provider call. Enforce
limits **before allocating or accumulating**; reject on overflow and keep the
previous committed state. Support backpressure with a bounded queue and
stop producing when the consumer is gone. Register cancellation outside the
service mutex, propagate it to the provider, then checkpoint only at safe
boundaries; guarantee exactly one terminal event. None of these controls or
streaming provider APIs are implemented yet.

### Voice transport (design only)

Voice remains off. Future capture requires explicit OS/browser microphone
permission, an obvious recording indicator and stop control; no automatic
capture on session resume. Prefer a separately scoped Rust STT/TTS bridge
with content-type validation, byte and duration ceilings (e.g. <= 2 MiB and
<= 60 seconds per recording), explicit provider consent, no raw-audio logs,
and no embedded provider tokens in WebView. Confirm codec and native WebView
support before choosing a recording format. Treat the transcript as untrusted
user input with the same text limits. Browser-to-provider WebSocket and JWT in
query strings are not part of this design. Voice is not a prerequisite for
safe text lessons.

### Display, edits and privacy (design only)

Start with escaped plain text and allowlisted structured question/answer
elements; never inject model-provided HTML, scripts, SVG or executable code.
If Markdown/formula rendering is added, sanitize its output and test hostile
links, HTML, diagrams and long blocks. Editing is a separate, explicit
user-initiated revision with backend validation, optimistic revision matching
and atomic replacement of a dedicated project-local Tutor document. Never
grant filesystem/shell tools to the model; running learner code requires an
independently verified OS sandbox and a new backend approval contract. No
Canvas/Excalidraw, code execution, RAG or auto-generated actions are enabled.

If persistence is introduced, put versioned Tutor lesson snapshots, progress
and transcripts in project-local storage separate from Chat-bot histories and
global launch metadata. Persist atomically; reject unknown versions without
overwriting; cap session count and total bytes; verify project/lesson identity
on every read and write; provide explicit deletion/retention semantics. Only
secret-free IDs and counts may enter launch state. Explain before sending
private lessons or transcripts to a configured remote LLM/STT/TTS provider;
no lesson content or audio should appear in logs. Currently **no Tutor progress
is stored**, including when the inactive card is clicked.

## Threat model and release gates

| Threat / trust boundary | Required mitigation before enabling a capability |
| --- | --- |
| Compromised WebView calling arbitrary IPC | Check opened project and Tutor session ownership, lesson revision, size and state in Rust on every operation. Tauri permissions constrain WebView exposure, not Rust bugs. |
| Prompt injection in lesson, answer or model output | Treat all content as data; never parse prose/JSON as privileged actions; no inherited Chat-bot tools. |
| Cross-project history leakage or corrupt/future JSON | Separate versioned store, strict identity check, atomic writes, explicit error and preservation of damaged originals. |
| HTML/diagram/code injection or arbitrary process execution | Text-only rendering first; sanitize structured formats; disable code execution without OS isolation. |
| Memory/CPU/cost exhaustion by prompts, streams or audio | Bound incoming and outgoing sizes, queues, duration, concurrency and model tokens at the backend. Cancellation must not corrupt checkpoints. |
| Microphone exposure / exfiltration to provider | User permission and indicator, bounded audio, provider disclosure, credential isolation and no raw-data logging. |

Before turning on text: add an isolated no-tools provider path, validated lesson
and session operations, budgeted provider execution, ownership/state/overflow
tests and a real cancel/stream contract. Before turning on voice, edits or
progress: complete their independent gates above. Until then, the explicit
inactive card is the correct UX, even for projects with working Chat-bot.

References: [Tauri IPC channels](https://v2.tauri.app/develop/calling-rust/#channels),
[Tauri capabilities](https://tauri.app/security/capabilities/),
[MDN WebSocket backpressure warning](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket),
[OWASP LLM Top 10](https://genai.owasp.org/llm-top-10/).
