# Desktop Coder: ограниченный read-only этап

Coder открывается из launcher после выбора проекта. Проводник отображает до 400 записей (глубина до 5), пропуская symlink, `.git`, `.aiagent`, `.venv`, `node_modules`, `target` и имена, похожие на секреты. Это **не** сканер секретов: обычные файлы и Git diff могут содержать секретные данные; не открывайте недоверенные проекты.

Git-изменения (`--porcelain=v1 -z --no-renames`) и diff конкретного файла доступны **только** если выбранный каталог — корень репозитория с локальным каталогом `.git`; подкаталог репозитория и linked worktree отклоняются. Доступны staged и unstaged diff tracked-файлов. Для untracked файлов diff пустой. Максимум 400 изменений, 64 KiB вывода status/check, 32 KiB diff, 5 секунд на Git. Пути с traversal, чувствительными компонентами и symlink не принимаются. Вывод ошибки Git не отправляется в WebView. Кнопка «Check whitespace» запускает только `git diff --check`, не запускает сборку/тесты или код проекта и не проверяет untracked файлы.

Панель `.venv` проверяет только наличие каталога и стандартного относительного интерпретатора. Shell activation не нужен для будущего запуска интерпретатора, но **исполнение Python, создание окружения, pip и произвольные проверки сейчас недоступны**. Отдельный backend-контракт должен привязать согласие пользователя к точной операции, выбрать sandbox с ограничением файлов и сети, ограничить время/вывод и перепроверять права до исполнения. `cwd`, список исполняемых программ и JS `window.confirm` сами по себе sandbox/подтверждением не являются.

Composer в Coder отключён: существующий Chat-bot endpoint не переключается автоматически на `coder.toml`, так как он не имеет backend confirmation для модельных правок. Существующие профили `.aiagent/agents/coder.toml` и project tool allowlists остаются доступными в CLI, но **Desktop Coder их пока не исполняет**. Текущие файловые checkpoints используются отдельными инструментами write/patch и содержат предыдущую версию файла; это не Git commit и не универсальный undo. `rollback_last_change` не восстанавливает любую операцию. Никакого разрешения на запись или rollback через Coder IPC нет.

Тонкие Tauri-команды `coder_tree`, `coder_changes`, `coder_diff`, `coder_check`, `coder_venv` вызывают общий `ApplicationService` и возвращают только ограниченные DTO. Проверки прав/пути выполняются в Rust, без filesystem/shell plugin permissions для WebView. Изменение файлов другими программами между проверкой пути и чтением (TOCTOU) и project-controlled Git config/attributes требуют дополнительных мер до объявления безопасности для недоверенного проекта.

## Проверка

```bash
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
npm --prefix frontend run check
npm --prefix frontend run build
```

Manual smoke: выбрать Coder и Git-root проекта; увидеть дерево и изменения; открыть staged/unstaged diff, выполнить whitespace check; убедиться, что prompt заблокирован. Повторить для каталога без `.git`, вложенного Git-каталога, ссылки и большого diff: операции должны возвращать recoverable ошибку без запуска кода.
