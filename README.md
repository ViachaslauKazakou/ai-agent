# Локальный AI-агент на Rust

Учебный проект локального аналога Cline / Aider / Claude Code. Цель проекта —
постепенно реализовать Rust-агента, который сможет подключаться к разным
языковым моделям через LiteLLM, анализировать проект и безопасно работать с
файлами.

Проект развивается по учебным этапам: от базового Rust и простой консольной
программы к CLI, HTTP-клиенту, инструментам и полноценному agent loop.

## Текущее состояние

Сейчас подключены CLI/REPL, LiteLLM и agent loop с безопасными файловыми tools:

- проект собирается через Cargo;
- библиотечная логика отделена от бинарной точки входа;
- программа читает одну строку из стандартного ввода и разбирает её на слова;
- подсчитывается общее и уникальное количество слов;
- выводится первое слово;
- пустой ввод обрабатывается через типизированную ошибку;
- добавлены `Role`, `Message` и `Session`;
- история сообщений хранится внутри сессии;
- добавлены CLI-аргументы и синхронный REPL;
- поддерживаются команды `/help`, `/status`, `/tools`, `/models`, `/config`, `/permissions`,
  `/index`, `/search`, `/model`, `/clear`, `/save`, `/load`, `/exit` и `/quit`;
- добавлены unit- и интеграционные тесты;
- добавлена конфигурация из defaults, `.env`, environment и CLI;
- настройки коннектора LLM (`LLM_PROVIDER`, base URL, API key, model и timeout)
  вынесены в `.env`;
- добавлены проверки путей и значений, а API key скрывается в диагностическом выводе;
- код снабжён комментариями и Rustdoc-документацией.

На текущем этапе это ещё не полноценный AI-агент: подключение к LLM, инструменты,
- агент умеет вызывать `list_directory`, `read_file`, `read_lines`, `search_files` и
  защищённый `write_file`;
- история автоматически сохраняется в `<working-dir>/.agent-session.json`; команды
  `/save` и `/load` позволяют управлять persistence вручную;
- `run_command` не включён по умолчанию и требует явного `command_allowlist` в
  `.agent.toml`;
- `confirm_writes = true` включает подтверждение записи в интерактивном REPL;
- вызовы инструментов выполняются циклом до `MAX_TOOL_ROUNDS`.
- добавлен локальный JSON-индекс `.agent/index.json` без embeddings и внешней БД;
- `project_search` ищет по chunks файлов, используя hash/mtime для инкрементального обновления;
- индекс исключает `.git`, `target`, `node_modules`, `.agent` и бинарные/слишком большие файлы.

## Структура проекта

Корень workspace:

```text
/Users/Viachaslau_Kazakou/Work/ai-agent/
```

Rust-проект:

```text
/Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent/
```

Основные файлы текущего этапа:

```text
ai-agent/
├── Cargo.toml
├── Cargo.lock
├── docs/
│   └── learning/
│       ├── 00-stage-0.md
│       ├── 01-rust-basics.md
│       ├── 02-modules-and-domain-types.md
│       ├── 03-cli.md
│       └── 04-errors-and-config.md
├── src/
│   ├── config.rs
│   ├── llm.rs
│   ├── cli.rs
│   ├── domain.rs
│   ├── error.rs
│   ├── lib.rs
│   ├── main.rs
│   └── word_processing.rs
└── tests/
    ├── cli_api.rs
    └── public_api.rs
```

## Требования

Для сборки проекта нужны:

- Rust toolchain;
- Cargo;
- macOS, Linux или Windows.

Проверить установку:

```bash
rustc --version
cargo --version
```

Проект разрабатывается с Rust edition 2024.

## Установка исполняемого файла

Чтобы запускать агент из каталога любого другого проекта, соберите и установите
исполняемый файл. После установки текущий каталог будет использоваться как
`working_dir`, поэтому команды нужно запускать из корня проекта, который агент
должен анализировать.

### Универсальный способ через Cargo

Из каталога репозитория выполните:

```bash
cargo install --path ai-agent
```

Если текущий каталог уже является каталогом Rust-проекта `ai-agent`, используйте:

```bash
cargo install --path .
```

Cargo соберёт release-версию и установит команду `ai-agent` в каталог Cargo bin.
После этого запуск из любого проекта выглядит так:

```bash
cd /path/to/another-project
ai-agent
```

Одноразовый prompt можно передать аргументом:

```bash
cd /path/to/another-project
ai-agent "Проанализируй структуру проекта и найди потенциальные проблемы"
```

Настройки конкретного проекта (`.env`, `.agent.toml`, `.aiagent/`) читаются из
текущей рабочей директории. Если запуск выполняется не из корня проекта,
укажите его явно:

```bash
ai-agent --working-dir /path/to/another-project
```

### macOS

После `cargo install` бинарник обычно находится в:

```text
~/.cargo/bin/ai-agent
```

Проверьте наличие каталога в `PATH`:

```bash
echo "$PATH"
```

Если `~/.cargo/bin` отсутствует, добавьте его в `~/.zshrc` для стандартного
macOS shell или в `~/.bashrc`, если используется Bash:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Примените изменения:

```bash
source ~/.zshrc
```

Проверка:

```bash
which ai-agent
ai-agent --help
```

Если Cargo устанавливался через `rustup`, каталог `~/.cargo/bin` обычно уже
добавляется в `PATH` автоматически. На macOS бинарник собирается для текущей
архитектуры (`arm64` на Apple Silicon или `x86_64` на Intel). Для запуска на
другой архитектуре нужен соответствующий Rust target или отдельная cross-build
настройка.

### Linux

Обычно Cargo также устанавливает бинарник в:

```text
~/.cargo/bin/ai-agent
```

Для Bash добавьте каталог в `~/.bashrc`, для Zsh — в `~/.zshrc`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Затем перезапустите shell или выполните, например:

```bash
source ~/.bashrc
which ai-agent
ai-agent --help
```

Для системной установки можно скопировать бинарник в каталог, уже находящийся
в `PATH`, например `/usr/local/bin`:

```bash
cargo build --release --manifest-path ai-agent/Cargo.toml
sudo install -m 755 ai-agent/target/release/ai-agent /usr/local/bin/ai-agent
```

Системная установка требует прав администратора и обычно менее удобна для
частых обновлений; для одного пользователя предпочтительнее `cargo install`.

### Windows

В PowerShell из корня репозитория выполните:

```powershell
cargo install --path .\ai-agent
```

Если команда выполняется из каталога `ai-agent`:

```powershell
cargo install --path .
```

Исполняемый файл будет установлен как:

```text
%USERPROFILE%\.cargo\bin\ai-agent.exe
```

Проверьте его запуск:

```powershell
Get-Command ai-agent
ai-agent.exe --help
```

Если PowerShell не находит команду, добавьте `%USERPROFILE%\.cargo\bin` в
пользовательскую переменную `Path` через **System Properties → Environment
Variables** или временно для текущего окна PowerShell:

```powershell
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
```

Для `cmd.exe` синтаксис запуска такой же:

```bat
cd C:\path\to\another-project
ai-agent.exe
```

Windows-версия использует файл `ai-agent.exe`; в командах Cargo имя пакета и
команды остаётся `ai-agent`.

### Сборка без установки

Если не нужно добавлять агент в `PATH`, соберите release-бинарник напрямую:

```bash
cargo build --release --manifest-path ai-agent/Cargo.toml
```

Результат находится в:

```text
macOS/Linux: ai-agent/target/release/ai-agent
Windows:     ai-agent\target\release\ai-agent.exe
```

Его можно запускать полным путём из другого проекта. Также можно создать
symlink на macOS/Linux:

```bash
ln -s "$PWD/ai-agent/target/release/ai-agent" "$HOME/.local/bin/ai-agent"
```

Убедитесь, что `$HOME/.local/bin` добавлен в `PATH`. На Windows вместо symlink
проще использовать `cargo install` или добавить каталог с `.exe` в `Path`.

### Обновление установленного агента

После изменений в исходном коде переустановите бинарник:

```bash
cargo install --path ai-agent --force
```

Или, если команда выполняется из каталога `ai-agent`:

```bash
cargo install --path . --force
```

Проверить версию/справку после обновления можно так:

```bash
ai-agent --help
```

## Как запустить консольное приложение

### Настройка LLM

Скопируйте `.env.example` в `.env` и укажите provider, endpoint, ключ и модель:

```bash
cp .env.example .env
# LITELLM_BASE_URL=http://localhost:4000/v1
# LITELLM_API_KEY=...
# MODEL=elite-gpt-5.6-luna
```

По умолчанию используется LiteLLM. Для локального Ollama, запущенного на
стандартном порту `11434`, достаточно указать:

```dotenv
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://localhost:11434/v1
MODEL=llama3.2
```

Ollama должен быть запущен (`ollama serve`), а модель — предварительно
загружена (`ollama pull llama3.2`). При необходимости можно использовать
`OLLAMA_API_KEY`, хотя локальный Ollama обычно не требует ключа. То же самое
можно выбрать разово: `cargo run -- --provider ollama --model llama3.2`.

Ключ используется только в Bearer-заголовке и не выводится в ошибки.

Перейти в каталог Rust-проекта:

```bash
cd /Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent
```

Запустить REPL через Cargo:

```bash
cargo run
```

Программа откроет синхронный REPL:

```text
Учебный AI-агент с LiteLLM. Введите /help для справки.
```

Доступны команды:

```text
/help
/status
/tools
/models
/agents
/agent
/agent reviewer
/skills
/skill testing
/config
/permissions
/model local-model
/clear
/save
/load
/exit
```

Обычный prompt отправляется модели. Агент может самостоятельно вызвать tools:

- `list_directory` — список entries рабочей директории;
- `read_file` — чтение UTF-8 файла с номерами строк;
- `write_file` — запись файла только при явном флаге `--allow-write`.

По умолчанию чтение и запись ограничены `--working-dir`; symlink escape и пути
выше рабочей директории отклоняются. Для режима изменения файлов:

```bash
cargo run -- --working-dir /path/to/project --allow-write "Создай notes.txt"
```

Обычный текст сохраняется как пользовательское сообщение:

```text
agent> Изучи структуру проекта
Prompt сохранён. Сообщений в истории: 1. Ответ LLM пока не подключён.
```

Для одноразового prompt используется позиционный аргумент:

```bash
cargo run -- "Изучи структуру проекта"
cargo run -- --model local-model --working-dir /tmp/project --verbose "Покажи статус"
```

Проверка инструментов без записи:

```bash
cargo run -- --working-dir /path/to/project "Прочитай README.md и объясни проект"
```

### Конфигурация разрешённых tools

Чтобы не указывать параметры безопасности при каждом запуске, создайте файл
`.agent.toml` в каталоге, из которого запускается программа. Шаблон находится в
`ai-agent/.agent.toml.example`:

```toml
[agent]
max_tool_rounds = 20
allow_write = false
enabled_tools = ["read_file", "list_directory"]
```

`enabled_tools` ограничивает tools, которые модель увидит и сможет вызвать.
Доступные значения включают `read_file`, `list_directory`, `write_file`,
`search_files`, `read_lines` и `project_search`.
Если файл отсутствует, используется безопасный список по умолчанию; запись всё
равно остаётся выключенной, пока `allow_write = true` не задан в конфиге или не
передан флаг `--allow-write`. Неизвестное имя tool останавливает запуск с
ошибкой конфигурации.

Конфигурация также проверяет, что `allow_write = true` используется только
вместе с `write_file` в `enabled_tools`.

Для другого файла конфигурации используйте:

```bash
cargo run -- --config /path/to/agent.toml "Прочитай README.md"
```

CLI-параметры имеют приоритет над соответствующими значениями TOML. `.env`
остаётся подходящим местом для API key и настроек подключения к LLM.

### Профили агентов и skills

Чтобы отделить настройки этого агента от других project-local конфигураций,
профили агентов и reusable skills хранятся в каталоге `.aiagent/`:

```text
.aiagent/
├── agents/
│   ├── reviewer.toml
│   └── writer.toml
└── skills/
    ├── testing/
    │   └── SKILL.md
    └── documentation/
        └── SKILL.md
```

#### Профиль агента

Каждый файл `.aiagent/agents/<name>.toml` описывает поведение отдельного
агента. Например:

```toml
description = "Агент для ревью изменений"
model = "review-model"
provider = "litellm"
system_prompt = "Проверяй изменения как внимательный code reviewer."
enabled_tools = ["read_file", "list_directory", "search_files", "read_lines"]
allow_write = false
confirm_writes = true
command_allowlist = []
max_tool_rounds = 20
skills = ["testing"]
```

Профиль может переопределить provider (`litellm` или `ollama`), модель, system prompt, список tools, permissions,
allowlist команд, лимит tool-раундов и подключённые skills. Endpoint, API key и
Endpoint, API key и рабочая директория остаются глобальными настройками приложения.
Если профиль выбирает Ollama, используется стандартный
`http://localhost:11434/v1`, когда глобальный provider — LiteLLM.

Встроенный профиль `default` создаётся автоматически из `.agent.toml`, `.env`
и CLI-параметров. Если каталог `.aiagent/` отсутствует, сохраняется прежнее
поведение агента с этим профилем.

#### Skills

Skill — это Markdown-инструкция в файле
`.aiagent/skills/<name>/SKILL.md`. Skill может описывать workflow, правила
анализа или требования к ответу:

```markdown
description: Проверка изменений тестами

# Testing

После изменений запускай подходящие unit- и integration-тесты.
```

Skills являются только инструкциями. Они не выполняют код, не добавляют tools и
не могут расширять permissions или `command_allowlist` профиля. Их содержимое
добавляется в system prompt агента после выбора профиля и подключённых skills.

#### Переключение в REPL

Список агентов и skills можно посмотреть командами:

```text
/agents        список доступных профилей
/agent         показать текущий профиль
/agent NAME    переключить профиль
/skills        список доступных skills
/skill NAME    активировать skill для текущего профиля
```

Переключение профиля не очищает историю `Session`. Меняются модель, system
prompt, доступные tools и permissions для следующих запросов. Если профиль или
skill не найден, REPL выводит ошибку и сохраняет текущий активный профиль.

Имена профилей должны совпадать с именем TOML-файла без расширения, а имена
skills — с именем каталога. Примеры файлов можно использовать как основу:

```text
ai-agent/.aiagent/agents/reviewer.toml.example
ai-agent/.aiagent/skills/testing/SKILL.md
```

Основные аргументы:

```bash
--model MODEL
--base-url URL
--working-dir PATH
--config PATH
--max-tool-rounds N
--request-timeout-secs SECONDS
-v, --verbose
PROMPT
```

### Конфигурация

Настройки объединяются в порядке `defaults → .env/environment → CLI`.
Пример переменных находится в `ai-agent/.env.example`. CLI-параметры имеют
приоритет над окружением; настройки подключения и API key хранятся в `.env`,
а API key не печатается в диагностическом выводе. Сейчас поддерживается
Поддерживаются `LLM_PROVIDER=litellm` (по умолчанию) и `LLM_PROVIDER=ollama`.
Оба провайдера используют OpenAI-compatible endpoint `/chat/completions`; для
Ollama default base URL — `http://localhost:11434/v1`.

Одноразовый режим не обращается к LLM:

```text
Ответ LLM пока не подключён: prompt сохранён в сессии.
```

### Запуск собранного бинарника

Сначала собрать проект:

```bash
cargo build
```

Затем запустить бинарный файл:

```bash
./target/debug/ai-agent
```

### Запуск и отладка в VS Code

В workspace уже добавлена конфигурация:

`/Users/Viachaslau_Kazakou/Work/ai-agent/.vscode/launch.json`

Для запуска:

1. Открыть workspace `/Users/Viachaslau_Kazakou/Work/ai-agent` в VS Code.
2. Открыть раздел **Run and Debug**.
3. Выбрать конфигурацию **Debug ai-agent**.
4. Нажать `F5`.

Конфигурация автоматически собирает проект из
`/Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent/Cargo.toml`, запускает бинарник
из `target/debug` и устанавливает `RUST_BACKTRACE=1`.

## Возможности текущей версии

### Доменная модель сессии

Публичная библиотека предоставляет следующие типы:

- `Role` — `System`, `User`, `Assistant`, `Tool`;
- `Message` — сообщение с ролью и непустым текстом;
- `Session` — UUID, рабочая директория, имя модели и история сообщений;
- `AppError` — типизированные ошибки пустого ввода, сообщения или имени модели.

CLI и REPL реализованы в `src/cli.rs`. Они управляют конфигурацией и историей
сессии, но пока не выполняют сетевые запросы, не вызывают LLM и не изменяют
файлы.

Пример использования библиотеки:

```rust
use ai_agent::{Message, Role, Session};

let mut session = Session::new("/tmp/project", "demo-model")?;
let message = Message::new(Role::User, "Изучи проект")?;
session.add_message(message);
```

Поля `Message` и `Session` приватны. Изменение состояния проходит через
методы, поэтому объект не может быть создан с пустым сообщением или моделью.

### Разделение тестов

Unit-тесты находятся рядом с реализацией модулей в `src/domain.rs` и
`src/word_processing.rs`. Интеграционные тесты находятся в
`tests/public_api.rs` и используют только публичный API библиотеки.

### Разбор текста

Программа использует `split_whitespace`, поэтому корректно обрабатывает:

- несколько пробелов между словами;
- табуляции;
- перевод строки;
- ведущие и завершающие пробелы.

### Подсчёт слов

Выводятся два значения:

- общее количество элементов в `Vec<String>`;
- количество уникальных слов через `HashSet`.

Слова сравниваются с учётом регистра: `Rust` и `rust` считаются разными
словами.

### Первое слово

Первое слово возвращается через `Option`. Это позволяет безопасно обработать
пустой список без обращения к несуществующему элементу.

### Обработка ошибок

Пустой ввод возвращается как `Result::Err` и отображается пользователю. Ошибка
чтения stdin также обрабатывается без вызова `panic!`.

### Тестирование

Текущая версия содержит 7 unit-тестов и 3 интеграционных теста для:

- разбора слов;
- пустого ввода;
- generic-функции подсчёта;
- получения первого элемента;
- подсчёта уникальных слов;
- создания и изменения сессии;
- валидации сообщений и имени модели;
- проверки публичного API библиотеки.

## Проверка качества кода

Все команды выполняются из каталога
`/Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent`.

Проверить форматирование:

```bash
cargo fmt -- --check
```

Проверить компиляцию:

```bash
cargo check
```

Запустить тесты:

```bash
cargo test
```

Запустить Clippy с запретом предупреждений:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Запустить все основные проверки одной последовательностью:

```bash
cargo fmt -- --check \
  && cargo check \
  && cargo test \
  && cargo clippy --all-targets --all-features -- -D warnings
```
