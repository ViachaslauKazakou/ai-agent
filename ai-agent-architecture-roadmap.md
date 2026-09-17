# AI Agent --- архитектурные замечания и roadmap

## 1. Цель проекта

Проект уже вышел за рамки простого CLI-клиента для LLM и фактически
движется в сторону локального coding/general-purpose agent.

Целевая архитектура:

``` text
                 ┌─────────────────────┐
                 │        CLI          │
                 │ REPL / commands     │
                 └──────────┬──────────┘
                            │
                            ▼
                 ┌─────────────────────┐
                 │       Agent         │
                 │                     │
                 │ orchestration       │
                 │ tool loop           │
                 │ context             │
                 └──────┬───────┬──────┘
                        │       │
               ┌────────▼─┐   ┌─▼───────────┐
               │   LLM    │   │    Tools    │
               │ Provider │   │  Registry   │
               └────┬─────┘   └─────┬──────┘
                    │               │
                 LiteLLM          filesystem
                 Ollama           search
                 OpenAI           git
                                   shell
                                   MCP
```

Главный принцип:

> Agent не должен знать деталей конкретного LLM provider или конкретной
> реализации tools.

------------------------------------------------------------------------

# 2. Текущее состояние

В проекте уже есть:

-   LiteLLM / Ollama через OpenAI-compatible API;
-   REPL и одноразовый prompt;
-   persistent sessions;
-   файловые tools;
-   поиск файлов;
-   project index;
-   agents и skills;
-   approval для операций записи;
-   ограничение `run_command`;
-   ограничение количества tool rounds;
-   unit/integration/HTTP tests;
-   Gmail / Outlook tools;
-   scheduler.

Поэтому сейчас не требуется переписывать проект с нуля.

Основная задача следующего этапа --- улучшить архитектурные границы
существующего кода.

------------------------------------------------------------------------

# 3. Главный архитектурный принцип

Систему стоит разделить на независимые уровни:

``` text
CLI
 │
 ▼
Agent
 │
 ├── Session
 ├── ContextManager
 ├── LlmProvider
 └── ToolRegistry
        │
        ├── Filesystem tools
        ├── Search tools
        ├── Git tools
        ├── Shell tools
        ├── Integrations
        └── MCP tools
```

Agent должен быть orchestration layer.

Он не должен содержать конкретную бизнес-логику:

``` text
read_file()
write_file()
search()
git()
gmail()
outlook()
```

Эта логика должна находиться в tools.

------------------------------------------------------------------------

# 4. ToolContext

Ввести отдельный `ToolContext`.

``` rust
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub permissions: ToolPermissions,
    pub limits: ToolLimits,
    pub cancellation: CancellationToken,
}
```

Например:

``` rust
pub struct ToolLimits {
    pub max_file_size: usize,
    pub max_output_size: usize,
    pub max_search_results: usize,
}
```

И:

``` rust
pub struct ToolPermissions {
    pub allow_write: bool,
    pub allow_command: bool,
}
```

## Почему это важно

Tool не должен получать весь `Agent` или `Session`.

Предпочтительная схема:

``` text
Agent
 │
 ├── Session
 ├── LLM
 └── ToolRegistry
          │
          └── ToolContext
```

а не:

``` text
Tool
 └── Agent
      └── Session
           └── everything
```

Это уменьшает связанность и упрощает дальнейшее появление sub-agents и
MCP.

------------------------------------------------------------------------

# 5. Tool abstraction

Разделить описание tool и его реализацию.

Рекомендуемый интерфейс:

``` rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolResult, ToolError>;
}
```

Пример:

``` rust
pub struct ReadFileTool;
```

Tool должен предоставлять:

-   имя;
-   описание;
-   JSON schema аргументов;
-   реализацию выполнения;
-   уровень риска.

------------------------------------------------------------------------

# 6. ToolDefinition

Рекомендуется добавить metadata:

``` rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub risk: ToolRisk,
}
```

Где:

``` rust
pub enum ToolRisk {
    ReadOnly,
    Write,
    Execute,
    External,
}
```

Пример:

``` text
read_file       ReadOnly
search_files    ReadOnly
git_diff        ReadOnly

write_file      Write
git_apply       Write

run_command     Execute
```

Это позволит PermissionManager принимать решения независимо от
конкретной реализации tool.

------------------------------------------------------------------------

# 7. ToolRegistry

Сделать отдельный `ToolRegistry`.

``` rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}
```

Основной API:

``` rust
impl ToolRegistry {
    pub fn register<T>(&mut self, tool: T)
    where
        T: Tool + 'static;

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;

    pub fn definitions(&self) -> Vec<ToolDefinition>;

    pub async fn execute(
        &self,
        call: &ToolCall,
        context: &ToolContext,
    ) -> ToolResult;
}
```

В результате Agent не должен знать, какие именно tools зарегистрированы.

------------------------------------------------------------------------

# 8. ToolResult

Не стоит ограничиваться:

``` rust
Result<String>
```

Лучше использовать структурированный результат:

``` rust
pub struct ToolResult {
    pub success: bool,
    pub content: String,
    pub data: Option<Value>,
    pub truncated: bool,
    pub metadata: ToolMetadata,
}
```

Например `read_file` может возвращать metadata:

``` json
{
  "path": "src/main.rs",
  "lines": 150,
  "truncated": false
}
```

а `content` будет содержать сам код.

Это также упростит будущий TUI.

------------------------------------------------------------------------

# 9. Ограничение размера Tool output

Особенно важно ограничивать вывод:

-   `read_file`;
-   `search_files`;
-   `list_directory`;
-   `run_command`;
-   тестов;
-   git-команд.

Например `cargo test` может вернуть десятки тысяч строк.

Tool должен уметь сообщать:

``` json
{
  "content": "...",
  "truncated": true,
  "total_bytes": 128394,
  "shown_bytes": 16000
}
```

В LLM нельзя бесконтрольно отправлять огромные результаты.

------------------------------------------------------------------------

# 10. Agent должен быть максимально простым

Рекомендуемая структура:

``` rust
pub struct Agent {
    pub llm: Box<dyn LlmProvider>,
    pub session: Session,
    pub tools: ToolRegistry,
    pub context: AgentContext,
}
```

Agent отвечает только за orchestration:

``` text
User
 ↓
Agent
 ↓
LLM
 ↓
tool call?
 ├── yes → ToolRegistry → ToolResult → LLM
 │
 └── no  → final answer
```

Основной loop концептуально:

``` rust
loop {
    let response = self.llm.complete(request).await?;

    if response.is_final() {
        return response;
    }

    for call in response.tool_calls {
        let result =
            self.tools.execute(&call, &self.tool_context).await?;

        self.session.add_tool_result(result);
    }
}
```

------------------------------------------------------------------------

# 11. AgentContext и ToolContext

Рекомендуется разделить эти два объекта.

## AgentContext

Состояние самого агента:

``` rust
pub struct AgentContext {
    pub project_dir: PathBuf,
    pub working_dir: PathBuf,
    pub model: String,
    pub max_tool_rounds: usize,
    pub mode: AgentMode,
}
```

## ToolContext

Только то, что разрешено конкретному tool:

``` rust
pub struct ToolContext {
    pub working_dir: PathBuf,
    pub permissions: ToolPermissions,
    pub limits: ToolLimits,
}
```

Это особенно полезно при появлении sub-agents.

------------------------------------------------------------------------

# 12. Session

Текущая концепция `Session` правильная:

``` text
id
working_dir
model
messages
```

Но не стоит разрешать всему приложению напрямую менять `messages`.

Вместо:

``` rust
session.messages.push(...)
```

использовать:

``` rust
session.add_user(...)
session.add_assistant(...)
session.add_tool_call(...)
session.add_tool_result(...)
```

------------------------------------------------------------------------

# 13. Message model

Не стоит хранить role как обычный `String`.

Вместо:

``` rust
pub struct Message {
    pub role: String,
    pub content: String,
}
```

лучше:

``` rust
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}
```

И message:

``` rust
pub struct Message {
    pub role: Role,
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}
```

Это станет особенно важно при полноценном tool calling.

------------------------------------------------------------------------

# 14. ToolCall

Рекомендуемая модель:

``` rust
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
```

Не смешивать tool call с обычным текстовым ответом модели.

------------------------------------------------------------------------

# 15. LLM Provider abstraction

Agent не должен работать непосредственно с OpenAI/LiteLLM JSON.

Нужен интерфейс:

``` rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse>;
}
```

Архитектура:

``` text
LiteLLM HTTP
     ↓
LiteLlmProvider
     ↓
CompletionResponse
     ↓
Agent
```

В будущем:

``` text
LlmProvider
 ├── LiteLlmProvider
 ├── OllamaProvider
 ├── OpenAiProvider
 └── OtherProvider
```

Agent при этом не изменяется.

------------------------------------------------------------------------

# 16. Provider response

Не стоит передавать в Agent raw OpenAI/LiteLLM response.

Использовать собственную модель:

``` rust
pub struct CompletionResponse {
    pub message: AssistantMessage,
    pub usage: Option<Usage>,
    pub finish_reason: FinishReason,
}
```

Это позволит подключать разные backend без изменения Agent.

------------------------------------------------------------------------

# 17. ContextManager

Это один из наиболее важных следующих компонентов.

Сейчас:

``` text
Session
 └── messages
```

Со временем история станет слишком большой.

Нужен:

``` rust
pub struct ContextManager {
    pub max_tokens: usize,
}
```

Схема:

``` text
Session history
       ↓
ContextManager
       ↓
messages suitable for LLM
```

В контекст должны попадать не обязательно все сообщения.

Например:

``` text
system instructions
+
project context
+
current task
+
recent conversation
+
important tool results
```

Вместо безусловного:

``` rust
messages.clone()
```

------------------------------------------------------------------------

# 18. PermissionManager

Approval не стоит реализовывать непосредственно внутри `write_file`.

Плохо:

``` text
WriteFileTool
 └── println!("Allow? [y/N]")
```

Лучше:

``` text
Agent
 ↓
PermissionManager
 ↓
Tool
```

Например:

``` rust
#[async_trait]
pub trait PermissionManager {
    async fn check(
        &self,
        action: Action,
    ) -> Result<Permission>;
}
```

Это позволит иметь:

``` text
CliPermissionManager
TuiPermissionManager
AutoPermissionManager
WebPermissionManager
```

без изменения tools.

------------------------------------------------------------------------

# 19. Почему PermissionManager особенно важен

Сейчас CLI может спрашивать пользователя.

Но в будущем появятся:

``` text
CLI
TUI
Web UI
API
autonomous mode
```

Tool не должен знать, откуда пользователь управляет агентом.

------------------------------------------------------------------------

# 20. Shell security

`run_command` должен рассматриваться как отдельный security boundary.

Рекомендуется:

``` text
read_file       ReadOnly
search_files    ReadOnly
git_diff        ReadOnly

write_file      Write

run_command     Execute
```

Команды вроде:

``` text
rm -rf
git reset --hard
sudo
curl
wget
```

должны иметь отдельную политику подтверждения или быть запрещены.

------------------------------------------------------------------------

# 21. Filesystem security

Все пути, передаваемые tools, должны быть проверены относительно
разрешённого `working_dir`.

Нельзя позволять:

``` text
../../etc/passwd
```

или аналогичным путям покидать project boundary.

Лучше централизовать проверку путей, а не реализовывать её независимо в
каждом filesystem tool.

------------------------------------------------------------------------

# 22. Project index

У проекта уже есть project index.

Его стоит оставить отдельным optional компонентом:

``` text
Project
 ├── filesystem
 ├── search
 └── index
```

Не следует делать index обязательным посредником между Agent и кодовой
базой.

В будущем могут одновременно существовать:

``` text
search_files
project_search
ripgrep
tree-sitter
semantic_search
embeddings
```

Agent должен видеть их как tools и выбирать нужный способ поиска.

------------------------------------------------------------------------

# 23. Не спешить с RAG

Для coding agent не нужно сразу добавлять:

``` text
embeddings
vector DB
semantic RAG
```

Сначала эффективнее построить:

``` text
filesystem
+
ripgrep/search
+
git
+
tree-sitter
```

И только когда станет понятна реальная проблема поиска по большим
репозиториям, добавлять semantic retrieval.

------------------------------------------------------------------------

# 24. Agents и Skills

У проекта уже есть agents и skills.

Стоит сохранить чёткое разделение.

## Agent

Имеет:

``` text
LLM
Session
Tools
Context
Agent loop
```

## Skill

Это набор инструкций и ресурсов:

``` text
instructions
+
optional resources
```

Например:

``` text
skills/
└── python/
    └── SKILL.md
```

## Specialized agent

Лучше считать конфигурацией обычного Agent:

``` toml
[agent]
name = "python-expert"
model = "qwen3-coder"

skills = [
    "python"
]

tools = [
    "read_file",
    "search_files",
    "write_file",
    "run_command"
]
```

Не создавать архитектуру вида:

``` text
Agent
 └── Agent
      └── Agent
```

без реальной необходимости.

------------------------------------------------------------------------

# 25. Gmail / Outlook

Gmail и Outlook tools не нужно удалять.

Но стоит структурировать tools:

``` text
tools/
├── filesystem/
│   ├── read_file.rs
│   ├── write_file.rs
│   ├── list_directory.rs
│   └── search.rs
│
├── git/
│   ├── status.rs
│   └── diff.rs
│
├── shell/
│   └── run_command.rs
│
└── integrations/
    ├── gmail/
    └── outlook/
```

Это отражает тот факт, что проект постепенно становится не только coding
agent, но и general-purpose local agent.

------------------------------------------------------------------------

# 26. MCP

MCP лучше добавлять после стабилизации локальной Tool abstraction.

Целевая структура:

``` text
ToolRegistry
 ├── LocalTool
 │    ├── ReadFileTool
 │    ├── WriteFileTool
 │    └── SearchTool
 │
 └── McpTool
      ├── GitHub
      ├── PostgreSQL
      ├── Browser
      └── ...
```

Agent при этом не должен меняться.

Именно поэтому важно сейчас правильно сделать `ToolRegistry`.

------------------------------------------------------------------------

# 27. TUI

TUI пока не является приоритетом.

Сначала должна работать архитектура:

``` text
Agent
 ├── LLM
 ├── Session
 ├── Context
 └── Tools
```

После этого можно сделать:

``` text
CLI
 └── Agent runtime

TUI
 └── Agent runtime

Web
 └── Agent runtime
```

Если approval и output tools не связаны напрямую с `println!`, переход к
TUI будет значительно проще.

------------------------------------------------------------------------

# 28. Рекомендуемая структура проекта

Итоговая структура может постепенно прийти к:

``` text
src/
│
├── main.rs
├── cli.rs
├── config.rs
├── error.rs
│
├── agent/
│   ├── mod.rs
│   ├── agent.rs
│   ├── context.rs
│   └── permissions.rs
│
├── session/
│   ├── mod.rs
│   ├── session.rs
│   └── message.rs
│
├── llm/
│   ├── mod.rs
│   ├── provider.rs
│   ├── types.rs
│   ├── litellm.rs
│   └── ollama.rs
│
├── tools/
│   ├── mod.rs
│   ├── registry.rs
│   ├── types.rs
│   ├── context.rs
│   │
│   ├── filesystem/
│   │   ├── read_file.rs
│   │   ├── write_file.rs
│   │   ├── list_directory.rs
│   │   └── search.rs
│   │
│   ├── git/
│   │   ├── status.rs
│   │   └── diff.rs
│   │
│   ├── shell/
│   │   └── run_command.rs
│   │
│   └── integrations/
│       ├── gmail/
│       └── outlook/
│
├── project/
│   ├── mod.rs
│   ├── index.rs
│   └── discovery.rs
│
└── skills/
    ├── loader.rs
    └── types.rs
```

Не нужно сразу механически переносить все файлы. Сначала выделить
интерфейсы и постепенно переместить реализацию.

------------------------------------------------------------------------

# 29. Рекомендуемый roadmap

## Commit 1 --- isolate tool context

Добавить:

``` text
ToolContext
ToolLimits
ToolPermissions
```

------------------------------------------------------------------------

## Commit 2 --- introduce ToolRegistry

Добавить:

``` text
Tool
ToolDefinition
ToolResult
ToolRegistry
```

------------------------------------------------------------------------

## Commit 3 --- separate LLM domain types

Добавить:

``` text
Role
Message
ToolCall
ToolDefinition
CompletionRequest
CompletionResponse
```

------------------------------------------------------------------------

## Commit 4 --- isolate Agent loop

Agent должен зависеть только от:

``` text
Session
LlmProvider
ToolRegistry
AgentContext
```

------------------------------------------------------------------------

## Commit 5 --- PermissionManager

Вынести из конкретных tools:

``` text
confirm_writes
allow_write
command_allowlist
```

------------------------------------------------------------------------

## Commit 6 --- ContextManager

Добавить управление:

``` text
token budget
recent messages
important tool results
project context
```

------------------------------------------------------------------------

## Commit 7 --- Git tools

Добавить:

``` text
git_status
git_diff
git_log
```

------------------------------------------------------------------------

## Commit 8 --- Streaming

Добавить streaming ответа LLM.

------------------------------------------------------------------------

## После этого

``` text
9. улучшение search
10. tree-sitter
11. persistent sessions
12. AGENTS.md / project instructions
13. MCP
14. sub-agents
15. semantic search / RAG
16. TUI
17. autonomous mode
```

------------------------------------------------------------------------

# 30. Что пока не стоит делать

Не стоит сейчас преждевременно добавлять:

``` text
embeddings
vector DB
semantic RAG
multi-agent
browser automation
autonomous mode
long-term memory
сложный planner
GUI
```

Сначала нужно довести до стабильного состояния цикл:

``` text
User
 ↓
Agent
 ↓
LLM
 ↓
Tool
 ↓
LLM
 ↓
Tool
 ↓
LLM
 ↓
Answer
```

------------------------------------------------------------------------

# 31. Целевая архитектура

В результате проект может выглядеть так:

``` text
                         ai-agent
                            │
              ┌─────────────┼─────────────┐
              │             │             │
             CLI            TUI           Web
              │             │             │
              └─────────────┼─────────────┘
                            │
                          Agent
                            │
              ┌─────────────┼──────────────┐
              ↓             ↓              ↓
             LLM          Context         Tools
              │             │              │
          LiteLLM       repository       ┌─┴───────────┐
          Ollama        history          │             │
          OpenAI       project          Local         MCP
                                         │             │
                                    ┌────┼────┐    ┌───┼────┐
                                    ↓    ↓    ↓    ↓   ↓    ↓
                                   FS   Git Shell GitHub DB Browser
```

Это позволит использовать проект не только как CLI coding agent, но и
как основу для экспериментов с локальными LLM, tools, skills, MCP,
memory и RAG.

------------------------------------------------------------------------

# 32. Ключевой критерий качества архитектуры

После каждого refactoring нужно стремиться к тому, чтобы добавление
нового инструмента не требовало изменений в Agent.

Например добавление:

``` text
PostgresTool
```

должно сводиться примерно к:

``` rust
registry.register(PostgresTool::new(...));
```

а не к изменению:

``` text
Agent
Session
LLM
REPL
Context
```

То же самое должно быть справедливо для MCP.

Если новый tool можно подключить через `ToolRegistry`, архитектура
движется в правильном направлении.

------------------------------------------------------------------------

# 33. Практический следующий шаг

Первым реальным refactoring стоит сделать не TUI и не новые tools, а:

``` text
Tool
ToolDefinition
ToolResult
ToolContext
ToolRegistry
```

Затем перевести существующие:

``` text
read_file
write_file
list_directory
search_files
read_lines
project_search
run_command
```

на этот интерфейс.

После этого станет намного проще оценить остальные части проекта и
добавлять новые возможности без архитектурного долга.
