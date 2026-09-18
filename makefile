PROJECT_DIR := .
MANIFEST := Cargo.toml
BINARY := ai-agent
INSTALL_DIR := $(HOME)/.cargo/bin

CARGO ?= cargo
ARGS ?=
OLLAMA_URL ?= http://127.0.0.1:11434/v1
OLLAMA_MODEL ?= llama3.2
LITELLM_URL ?= http://127.0.0.1:4000/v1
LITELLM_MODEL ?= openai/gpt-4o-mini

.PHONY: help build release install uninstall run run-release run-installed \
	run-ollama run-litellm run-ollama-release run-litellm-release \
	run-ollama-installed run-litellm-installed check test fmt fmt-check clippy clean

help:
	@printf '%s\n' \
		'make build          - debug-сборка' \
		'make release        - release-сборка' \
		'make install        - собрать и установить ai-agent в ~/.cargo/bin' \
		'make uninstall      - удалить установленный ai-agent' \
		'make run            - запустить debug-версию через Cargo' \
		'make run-release    - собрать и запустить release-версию' \
		'make run-installed  - запустить установленную команду ai-agent' \
		'make run-ollama     - запустить debug-версию с Ollama' \
		'make run-litellm    - запустить debug-версию с LiteLLM' \
		'make run-ollama-release  - запустить release-версию с Ollama' \
		'make run-litellm-release - запустить release-версию с LiteLLM' \
		'make run-ollama-installed  - запустить установленный ai-agent с Ollama' \
		'make run-litellm-installed - запустить установленный ai-agent с LiteLLM' \
		'make check          - проверить компиляцию' \
		'make test           - запустить тесты' \
		'make fmt            - отформатировать код' \
		'make fmt-check      - проверить форматирование' \
		'make clippy         - запустить Clippy' \
		'make clean          - удалить target'

build:
	$(CARGO) build --manifest-path $(MANIFEST)

release:
	$(CARGO) build --release --manifest-path $(MANIFEST)

install:
	$(CARGO) install --path . --locked --force

uninstall:
	rm -f $(INSTALL_DIR)/$(BINARY)

run:
	$(CARGO) run --manifest-path $(MANIFEST) -- $(ARGS)

run-release: release
	target/release/$(BINARY) $(ARGS)

run-installed:
	$(BINARY) $(ARGS)

run-ollama:
	LLM_PROVIDER=ollama OLLAMA_BASE_URL=$(OLLAMA_URL) MODEL=$(OLLAMA_MODEL) \
		$(CARGO) run --manifest-path $(MANIFEST) -- $(ARGS)

run-litellm:
	LLM_PROVIDER=litellm LITELLM_BASE_URL=$(LITELLM_URL) MODEL=$(LITELLM_MODEL) \
		$(CARGO) run --manifest-path $(MANIFEST) -- $(ARGS)

run-ollama-release: release
	LLM_PROVIDER=ollama OLLAMA_BASE_URL=$(OLLAMA_URL) MODEL=$(OLLAMA_MODEL) \
		target/release/$(BINARY) $(ARGS)

run-litellm-release: release
	LLM_PROVIDER=litellm LITELLM_BASE_URL=$(LITELLM_URL) MODEL=$(LITELLM_MODEL) \
		target/release/$(BINARY) $(ARGS)

run-ollama-installed:
	LLM_PROVIDER=ollama OLLAMA_BASE_URL=$(OLLAMA_URL) MODEL=$(OLLAMA_MODEL) \
		$(BINARY) $(ARGS)

run-litellm-installed:
	LLM_PROVIDER=litellm LITELLM_BASE_URL=$(LITELLM_URL) MODEL=$(LITELLM_MODEL) \
		$(BINARY) $(ARGS)

check:
	$(CARGO) check --manifest-path $(MANIFEST)

test:
	$(CARGO) test --manifest-path $(MANIFEST)

fmt:
	$(CARGO) fmt --manifest-path $(MANIFEST)

fmt-check:
	$(CARGO) fmt --manifest-path $(MANIFEST) -- --check

clippy:
	$(CARGO) clippy --manifest-path $(MANIFEST) --all-targets --all-features -- -D warnings

clean:
	$(CARGO) clean --manifest-path $(MANIFEST)

git:
	git checkout main
	git pull origin main
	git status
