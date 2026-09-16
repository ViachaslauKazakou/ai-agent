# Локальный AI-агент на Rust

Учебный проект локального аналога Cline / Aider / Claude Code. Цель проекта —
постепенно реализовать Rust-агента, который сможет подключаться к разным
языковым моделям через LiteLLM, анализировать проект и безопасно работать с
файлами.

Проект развивается по учебным этапам: от базового Rust и простой консольной
программы к CLI, HTTP-клиенту, инструментам и полноценному agent loop.

## Текущее состояние

Сейчас завершены этапы 0, 1, 2, 3 и 4:

- проект собирается через Cargo;
- библиотечная логика отделена от бинарной точки входа;
- программа читает одну строку из стандартного ввода и разбирает её на слова;
- подсчитывается общее и уникальное количество слов;
- выводится первое слово;
- пустой ввод обрабатывается через типизированную ошибку;
- добавлены `Role`, `Message` и `Session`;
- история сообщений хранится внутри сессии;
- добавлены CLI-аргументы и синхронный REPL;
- поддерживаются команды `/help`, `/status`, `/model`, `/clear`, `/exit` и `/quit`;
- добавлены unit- и интеграционные тесты;
- добавлена конфигурация из defaults, `.env`, environment и CLI;
- добавлены проверки путей и значений, а API key скрывается в диагностическом выводе;
- код снабжён комментариями и Rustdoc-документацией.

На текущем этапе это ещё не полноценный AI-агент: подключение к LLM, инструменты,
работа с файлами и agent loop будут реализованы на следующих этапах.

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

## Как запустить консольное приложение

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
Учебный AI-агент без LLM. Введите /help для справки.
```

Доступны команды:

```text
/help
/status
/model local-model
/clear
/exit
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

Основные аргументы:

```bash
--model MODEL
--base-url URL
--working-dir PATH
--max-tool-rounds N
--request-timeout-secs SECONDS
-v, --verbose
PROMPT
```

### Конфигурация

Настройки объединяются в порядке `defaults → .env/environment → CLI`.
Пример переменных находится в `ai-agent/.env.example`. CLI-параметры имеют
приоритет над окружением; API key не печатается в диагностическом выводе.

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

## Учебная документация

Подробные объяснения реализации находятся в каталоге:

`/Users/Viachaslau_Kazakou/Work/ai-agent/ai-agent/docs/learning/`

- `00-stage-0.md` — подготовка Cargo-проекта и чистая точка входа;
- `01-rust-basics.md` — ownership, borrowing, `String`, `&str`, `Vec`,
  `Option`, `Result`, generic-функции и unit-тесты.
- `02-modules-and-domain-types.md` — модули, `enum`, `struct`, методы,
  собственные ошибки, сессия и интеграционные тесты.
- `03-cli.md` — аргументы `clap`, одноразовый prompt и команды REPL.
- `04-errors-and-config.md` — `thiserror`, `.env`, приоритет источников,
  валидация и безопасная обработка ошибок.

Каждый следующий этап должен иметь собственную учебную заметку с описанием
теории, изменённых файлов, тестов, команд проверки и критериев завершения.

## Ограничения текущей версии

Пока программа:

- не подключается к языковой модели;
- не использует LiteLLM, Ollama или OpenAI API;
- не читает и не изменяет файлы проекта;
- не выполняет shell-команды или Git-операции;
- не является автономным coding agent.

При этом доменные типы `Session`, `Message` и `Role` уже подготовлены для
следующих этапов и сейчас используются CLI/REPL только для хранения prompt-ов;
ответы модели и инструменты появятся позже.

Эти ограничения намеренные: проект реализуется поэтапно в учебных целях.

## План развития

Следующие этапы:

1. **Ошибки и конфигурация** — `.env`, настройки и расширение `AppError`.
2. **Файлы и безопасность путей** — безопасное чтение и список директорий.
4. **Async/Tokio** — асинхронные операции и понимание runtime.
5. **HTTP/JSON/LiteLLM** — запросы к модели и типизированные ответы.
6. **Session и provider traits** — история сообщений и fake provider.
7. **Tools и registry** — подключаемые инструменты.
8. **Agent loop** — prompt → LLM → tool call → result → answer.
9. **Запись файлов и подтверждения** — безопасные изменения проекта.
10. **Тестирование и production-практики** — tracing, интеграционные тесты и
    документация.

---

## Проектная концепция будущего агента

Ниже описана целевая архитектура, которая будет реализовываться постепенно.


Я бы не начинал с UI и не пытался сразу делать полноценного агента. Сначала построил бы правильное ядро.

                    ┌──────────────────────┐
                    │       CLI            │
                    │  repl / commands     │
                    └──────────┬───────────┘
                               │
                    ┌──────────▼───────────┐
                    │      Session         │
                    │ history / context    │
                    └──────────┬───────────┘
                               │
              ┌────────────────▼────────────────┐
              │             Agent               │
              │                                  │
              │  prompt → model → tool → ...     │
              └───────┬──────────────┬───────────┘
                      │              │
              ┌───────▼──────┐ ┌────▼───────────┐
              │  LLM Client  │ │     Tools      │
              │              │ │                 │
              │ LiteLLM/API  │ │ read_file       │
              │ Ollama       │ │ write_file      │
              │ OpenAI       │ │ list_files      │
              └──────────────┘ │ grep/search     │
                               │ shell           │
                               │ git             │
                               └─────────────────┘

local-agent/
├── Cargo.toml
├── .env
├── .gitignore
└── src/
    ├── main.rs
    ├── cli.rs
    ├── config.rs
    ├── session.rs
    ├── agent.rs
    ├── error.rs
    │
    ├── llm/
    │   ├── mod.rs
    │   ├── types.rs
    │   └── litellm.rs
    │
    └── tools/
        ├── mod.rs
        └── types.rs


Первый набор tools

read_file
write_file
list_directory
search

Потом:

replace_in_file
run_command
git_diff
git_status
git_log

```
loop {
    let response = llm.complete(&messages).await?;

    match response {
        Response::Text(text) => {
            println!("{text}");
            break;
        }

        Response::ToolCall(call) => {
            let result = tools.execute(call).await?;

            messages.push(call);
            messages.push(tool_result(result));
        }
    }
}
```

                 ┌──────────────┐
                 │     User     │
                 └──────┬───────┘
                        ↓
                 ┌──────────────┐
                 │     Agent    │
                 └──────┬───────┘
                        ↓
                 ┌──────────────┐
                 │      LLM     │
                 └──────┬───────┘
                        ↓
                 ┌──────────────┐
                 │ Tool call?   │
                 └───┬──────┬───┘
                    yes      no
                     ↓        ↓
                  Tool       answer
                     │
                     ↓
                  result
                     │
                     └──────→ LLM


9. Сессия

Тебе нужна отдельная сущность:

struct Session {
    id: Uuid,
    working_dir: PathBuf,
    messages: Vec<Message>,
    model: String,
}