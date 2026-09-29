PROJECT_DIR := .
ROOT_DIR := $(abspath $(PROJECT_DIR))
MANIFEST := Cargo.toml
BINARY := ai-agent
INSTALL_DIR := $(HOME)/.cargo/bin

CARGO ?= cargo
ARGS ?=
OLLAMA_URL ?= http://127.0.0.1:11434/v1
OLLAMA_MODEL ?= llama3.2
LITELLM_URL ?= http://127.0.0.1:4000/v1
LITELLM_MODEL ?= openai/gpt-4o-mini

.PHONY: help build release release-all desktop-build desktop-install desktop-uninstall install uninstall run run-release run-installed \
	run-ollama run-litellm run-ollama-release run-litellm-release \
	run-ollama-installed run-litellm-installed check test fmt fmt-check clippy clean

help:
	@printf '%s\n' \
		'make build          - debug-сборка' \
		'make release        - release-сборка' \
		'make release-all    - release-сборка консоли и desktop' \
		'make desktop-build  - release-сборка desktop-клиента via-agent' \
		'make desktop-install - собрать и установить desktop-команду via-agent' \
		'make desktop-uninstall - удалить desktop-команду via-agent' \
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

release-all: release desktop-build

desktop-build:
	@test -d $(PROJECT_DIR)/frontend/node_modules/@tauri-apps/plugin-dialog || (echo "frontend dependencies are missing or outdated; running npm install" && cd $(PROJECT_DIR)/frontend && npm install)
	cd $(PROJECT_DIR)/src-tauri && RUST_BACKTRACE=1 $(CARGO) tauri build

desktop-install: desktop-build
	@mkdir -p $(INSTALL_DIR)
	@case "$$(uname -s)" in \
		Darwin) \
			printf '%s\n' '#!/bin/sh' 'exec "$(ROOT_DIR)/src-tauri/target/release/bundle/macos/via-agent.app/Contents/MacOS/via-agent" "$$@"' > "$(INSTALL_DIR)/via-agent"; \
			;; \
		*) \
			printf '%s\n' '#!/bin/sh' 'exec "$(ROOT_DIR)/src-tauri/target/release/via-agent" "$$@"' > "$(INSTALL_DIR)/via-agent"; \
			;; \
	esac
	@chmod +x $(INSTALL_DIR)/via-agent
	@printf '%s\n' "Installed via-agent to $(INSTALL_DIR)/via-agent"

desktop-uninstall:
	rm -f $(INSTALL_DIR)/via-agent

install:
	$(CARGO) install --path . --locked --force

uninstall:
	rm -f $(INSTALL_DIR)/$(BINARY)

run:
	$(CARGO) run --manifest-path $(MANIFEST) -- $(ARGS)

desktop:
	@test -d $(PROJECT_DIR)/frontend/node_modules/@tauri-apps/plugin-dialog || (echo "frontend dependencies are missing or outdated; running npm install" && cd $(PROJECT_DIR)/frontend && npm install)
	cd $(PROJECT_DIR)/src-tauri && RUST_BACKTRACE=1 $(CARGO) tauri dev

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
	$(CARGO) check --workspace

test:
	$(CARGO) test --workspace

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

clippy:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

clean:
	$(CARGO) clean --workspace

git:
	git checkout main
	git pull origin main
	git status
