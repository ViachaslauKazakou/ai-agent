# Этап 4: ошибки и конфигурация

На этом этапе приложение получает типизированную конфигурацию и единый поток
ошибок. Источники объединяются в порядке `defaults → .env/environment → CLI`;
CLI имеет наивысший приоритет.

## `Config`

`src/config.rs` содержит `provider`, `api_base_url`, `api_key`, `model`, `working_dir`,
`max_tool_rounds`, `request_timeout_secs`, `log_level` и `verbose`.

`LLM_PROVIDER` по умолчанию равен `litellm`; неизвестные значения отклоняются.
`Config::load` вызывает `dotenvy::dotenv`, затем читает окружение. Для
детерминированных тестов `Config::from_sources` принимает карту переменных
явно. Рабочая директория преобразуется в абсолютный путь и проверяется как
существующий каталог. Значения `MAX_TOOL_ROUNDS` и `REQUEST_TIMEOUT_SECS`
должны быть положительными.

API key сохраняется для будущего HTTP-клиента, но заменяется на `<redacted>` в
`Debug`-выводе `Config` и не включается в ошибки.

## Каталог ошибок

`AppError` в `src/error.rs` реализован через `thiserror`:

| Вариант | Пример сообщения | Где возникает |
|---|---|---|
| `EmptyInput` | `Ввод пустой: нужно указать хотя бы одно слово.` | учебный ввод |
| `EmptyMessage` | `Сообщение не может быть пустым.` | создание `Message` |
| `EmptyModel` | `Имя модели не может быть пустым.` | создание сессии и `Config` |
| `InvalidConfig` | `Некорректная конфигурация: ...` | проверка лимитов и URL |
| `InvalidEnvironmentValue` | `Некорректное значение переменной MAX_TOOL_ROUNDS: many` | разбор environment |
| `EnvironmentFile` | `Не удалось загрузить файл окружения: ...` | повреждённый `.env` |
| `InvalidWorkingDirectory` | `Рабочая директория недоступна: ...` | проверка пути |

Конфигурационный слой возвращает `Result<Config, AppError>` и передаёт ошибки
оператором `?`. `main` является границей приложения: печатает ошибку в
`stderr` и завершает программу без `panic`. REPL обрабатывает ошибку отдельной
команды и продолжает работу.

## Проверка

```bash
cargo fmt -- --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Тесты проверяют defaults, приоритет CLI над environment, пустое имя модели,
ошибочные числовые значения, несуществующие пути и отсутствие API key в
`Debug`-выводе.

## Контракт провайдера

`src/llm.rs` содержит сериализуемые Chat Completions-типы,
`CompletionRequest`, `CompletionResponse`, асинхронный trait `LlmProvider` и
реализацию `LiteLlmProvider` через `reqwest`. Одноразовый prompt и сообщения
REPL отправляются на `{LITELLM_BASE_URL}/chat/completions`. Ответ assistant
добавляется в историю сессии. Tool calls распознаются, но выполнение
инструментов относится к следующему этапу.