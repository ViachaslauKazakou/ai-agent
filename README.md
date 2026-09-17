# Локальный AI-агент на Rust

Консольный AI-агент с OpenAI-compatible клиентами для LiteLLM и Ollama. Агент
поддерживает интерактивный REPL, project-local профили и skills, безопасные
инструменты для работы с файлами, локальный индекс проекта и цикл
`LLM → tool → результат → LLM`.

## Возможности

- LiteLLM и Ollama через `/chat/completions`;
- одноразовый prompt и интерактивный REPL;
- сохранение истории в `<working-dir>/.agent-session.json`;
- tools `read_file`, `list_directory`, `write_file`, `search_files`,
  `read_lines`, `project_search` и опциональный `run_command`;
- ограничение tools списком `enabled_tools`;
- запись файлов выключена без явного `allow_write = true`;
- подтверждение записи в интерактивном режиме через `confirm_writes`;
- `run_command` работает только для команд из `command_allowlist`;
- ограничение agent loop через `max_tool_rounds`;
- переключение tools во время REPL командами `/tools on` и `/tools off`;
- project-local агенты в `.aiagent/agents/*.toml`;
- project-local skills в `.aiagent/skills/<name>/SKILL.md`;
- инкрементальный JSON-индекс проекта в `.agent/index.json`;
- unit-, integration- и HTTP-клиентские тесты.

### Outlook / Microsoft Graph

Для read-only email tools создайте public client app registration в Microsoft Entra,
задайте `MICROSOFT_GRAPH_CLIENT_ID` и выполните:

```bash
cargo run -- --graph-login
```

Device-code flow выводит URL и код в терминал. Refresh token сохраняется только в
системном credential store (macOS Keychain, Windows Credential Manager или Linux
Secret Service); в `.env`, `.agent-session.json` и логах он не записывается.
Для CI/mock HTTP допускается временный `MICROSOFT_GRAPH_ACCESS_TOKEN`.

`get_email` и email search results передаются текущему LLM, но помечены ephemeral и
не сохраняются в persistent session history. Scheduler запускается так:

```bash
cargo run -- --scheduler --schedule-file .aiagent/schedules.toml
```

Остановка через Ctrl-C выполняется корректно между job; job выполняются строго
последовательно.

> Для моделей, которые не поддерживают tool definitions, перед prompt используйте
> `/tools off`. Автоматический retry после HTTP 400 пока не реализован.

## Требования

- Rust 1.89 или новее;
- Cargo;
- macOS, Linux или Windows;
- запущенный LiteLLM или Ollama для реальных запросов.

Проверка toolchain:

```bash
rustc --version
cargo --version
```

Проект использует Rust edition 2024 и находится в каталоге
`/Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent`.

## Сборка и установка

Команды `make` запускаются из корня репозитория:

```bash
make build       # debug-сборка
make release     # release-сборка
make install     # установка ai-agent в ~/.cargo/bin
make uninstall   # удаление установленного бинарника
```

После установки запускать агент можно из корня любого анализируемого проекта:

```bash
cd /path/to/project
ai-agent
```

Без установки:

```bash
cd /Users/Viachaslau_Kazakou/Work/ai-agent
make run
# или
cargo run --manifest-path ai-agent/Cargo.toml
```

Рабочую директорию можно задать явно:

```bash
ai-agent --working-dir /path/to/project
```

Основные CLI-аргументы:

```text
--provider PROVIDER
--model MODEL
--base-url URL
--working-dir PATH
--config PATH
--max-tool-rounds N
--request-timeout-secs SECONDS
--allow-write
-v, --verbose
PROMPT
```

Если `PROMPT` не указан, запускается REPL. Если prompt указан, выполняется один
запрос с tools, включёнными по умолчанию:

```bash
ai-agent --provider ollama --model llama3.2 "Покажи структуру проекта"
```

## Подключение к Ollama

Установите и запустите Ollama, затем загрузите модель:

```bash
ollama serve
ollama pull llama3.2
```

Запуск через Makefile:

```bash
make run-ollama
make run-ollama OLLAMA_MODEL=qwen2.5-coder:1.5b-base
```

Параметры Makefile по умолчанию:

```text
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://127.0.0.1:11434/v1
MODEL=llama3.2
```

То же через CLI или переменные окружения:

```bash
LLM_PROVIDER=ollama \
OLLAMA_BASE_URL=http://127.0.0.1:11434/v1 \
MODEL=llama3.2 \
  ai-agent
```

Локальному Ollama API key обычно не нужен. Если endpoint его требует, задайте
`OLLAMA_API_KEY`.

### Модели без поддержки tools

В REPL отключите definitions до отправки запроса:

```text
/tools off
Опиши структуру проекта без вызова инструментов
```

Проверить или снова включить настройку можно так:

```text
/tools
/tools on
```

`/tools off` влияет только на будущие запросы. История сообщений и уже
сохранённые tool calls не удаляются.

## Подключение к LiteLLM

LiteLLM должен предоставлять OpenAI-compatible endpoint, например
`http://127.0.0.1:4000/v1`.

```bash
make run-litellm
```

Параметры Makefile по умолчанию:

```text
LLM_PROVIDER=litellm
LITELLM_BASE_URL=http://127.0.0.1:4000/v1
MODEL=openai/gpt-4o-mini
```

Для защищённого endpoint задайте ключ:

```bash
LITELLM_API_KEY='your-key' \
  make run-litellm \
  LITELLM_URL=http://127.0.0.1:4000/v1 \
  LITELLM_MODEL=openai/gpt-4o-mini
```

Также доступны `make run-ollama-installed`, `make run-litellm-installed`,
`make run-ollama-release` и `make run-litellm-release`.

## REPL

Запуск:

```bash
ai-agent
```

Команды:

```text
/help          показать справку
/status        показать состояние сессии
/tools         показать состояние tools
/tools on|off  включить или выключить tools для следующих запросов
/models        получить список моделей endpoint
/agents        показать project-local профили
/agent         показать текущий профиль
/agent NAME    переключить профиль
/skills        показать доступные skills
/skill NAME    активировать skill текущего профиля
/config        показать конфигурацию без API key
/permissions   показать permissions текущего профиля
/index         построить или обновить индекс проекта
/index status  показать состояние индекса
/search QUERY  поиск по индексированным фрагментам
/model         показать текущую модель
/model NAME    изменить модель сессии
/stats         показать настройки статистики
/stats on|off  включить или выключить токены и время ответа
/clear         очистить историю
/save          сохранить историю
/load          загрузить историю
/exit, /quit   выйти
```

Любой текст, не начинающийся с `/`, отправляется как пользовательский prompt.

## Tools и безопасность

| Tool | Назначение |
| --- | --- |
| `read_file` | чтение файла |
| `list_directory` | список каталога |
| `write_file` | запись файла внутри `working_dir` |
| `search_files` | поиск текста по файлам |
| `read_lines` | чтение диапазона строк |
| `project_search` | поиск по локальному индексу |
| `run_command` | запуск явно разрешённой команды |

Пути проверяются и не могут выйти за пределы `working_dir`. Файлы ограничены
по размеру, результаты tools могут быть обрезаны. `run_command` не входит в
конфигурацию по умолчанию; для него нужно одновременно добавить tool в
`enabled_tools` и команды в `command_allowlist`.

## Конфигурация

В Rust-проекте скопируйте пример окружения:

```bash
cd /Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent
cp .env.example .env
```

Поддерживаемые переменные:

| Переменная | Назначение | Значение по умолчанию |
| --- | --- | --- |
| `LLM_PROVIDER` | `litellm` или `ollama` | `litellm` |
| `LITELLM_BASE_URL` | LiteLLM base URL | `http://localhost:4000/v1` |
| `OLLAMA_BASE_URL` | Ollama base URL | `http://localhost:11434/v1` |
| `LITELLM_API_KEY` | ключ LiteLLM | отсутствует |
| `OLLAMA_API_KEY` | ключ Ollama | отсутствует |
| `MODEL` | имя модели | `demo-model` |
| `WORKING_DIR` | рабочий каталог | `.` |
| `MAX_TOOL_ROUNDS` | максимум раундов tools | `20` |
| `REQUEST_TIMEOUT_SECS` | HTTP timeout | `120` |
| `RUST_LOG` | уровень логирования | `info` |

Источники объединяются в порядке:

```text
defaults → .env/environment → CLI
```

CLI имеет наивысший приоритет. Для API key используется provider-specific
переменная. Команда `/config` и `--verbose` скрывают значение ключа.

### `.agent.toml`

Файл `.agent.toml` находится в `working_dir`. Пример — `.agent.toml.example`:

```toml
[agent]
max_tool_rounds = 20
allow_write = false
enabled_tools = ["read_file", "list_directory", "search_files", "read_lines", "project_search"]
command_allowlist = []
confirm_writes = true
```

`allow_write = true` разрешает `write_file`, но tool всё равно должен быть в
`enabled_tools`. При `confirm_writes = true` интерактивный REPL запрашивает
подтверждение перед записью.

### Профили агентов и skills

Project-local настройки хранятся в `.aiagent/`:

```text
.aiagent/
├── agents/
│   └── reviewer.toml
└── skills/
    └── testing/
        └── SKILL.md
```

Профиль задаёт модель, provider, system prompt, tools, permissions, лимит
tool rounds и список skills. Профиль по умолчанию создаётся автоматически.
Пример профиля находится в
`ai-agent/.aiagent/agents/reviewer.toml.example`, пример skill — в
`ai-agent/.aiagent/skills/testing/SKILL.md`.

Skills являются инструкциями для system prompt: они не добавляют tools и не
расширяют permissions или `command_allowlist`.

### Индекс проекта

Команда `/index` создаёт или обновляет `.agent/index.json`. Индекс хранит
текстовые chunks и используется tool `project_search`; embeddings и внешняя
база данных не требуются. Исключаются `.git`, `target`, `node_modules`, `.agent`,
`.agent-session.json`, бинарные и слишком большие файлы.

## Структура

```text
ai-agent/
├── Cargo.toml
├── .env.example
├── .agent.toml.example
├── .aiagent/
├── docs/learning/
├── src/
│   ├── agent.rs       # agent loop
│   ├── agents.rs      # profiles и skills
│   ├── cli.rs         # CLI и REPL parser
│   ├── config.rs      # конфигурация
│   ├── index.rs       # локальный индекс
│   ├── llm.rs         # LiteLLM/Ollama client
│   ├── tools.rs       # tools и permissions
│   └── main.rs        # бинарная точка входа
└── tests/
```

## Проверки

Из каталога `ai-agent`:

```bash
cargo fmt -- --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Или из корня репозитория:

```bash
make fmt-check
make check
make test
make clippy
```

Полная последовательность:

```bash
cargo fmt -- --check \
  && cargo check \
  && cargo test \
  && cargo clippy --all-targets --all-features -- -D warnings
```

## Текущие ограничения

- автоматический fallback/retry без tools после HTTP 400 пока не реализован;
- tools выключаются вручную через `/tools off`;
- индекс — локальный JSON-поиск без embeddings;
- агент рассчитан на OpenAI-compatible `/chat/completions`.