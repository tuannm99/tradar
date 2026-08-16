DOCKER_SERVICES := postgres redis mongo elasticsearch5 elasticsearch7 elasticsearch8 clickhouse cassandra rabbitmq kafka

.PHONY: help build run fmt fmt-check clippy check \
	test test-unit test-docker \
	test-core test-connector-spi test-workbench test-app \
	test-sqlite test-postgres test-redis test-mongo test-elasticsearch test-http \
	up down ps

CARGO := cargo
COMPOSE := docker compose

help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "  %-20s %s\n", $$1, $$2}'
	@printf "  %-20s %s\n" "up-<service>" "Start one docker-compose service, e.g. \`make up-postgres\`"
	@printf "  %-20s %s\n" "down-<service>" "Stop and remove one docker-compose service, e.g. \`make down-postgres\`"
	@printf "  %-20s %s\n" "" "  -> $(DOCKER_SERVICES)"

build: ## cargo build --workspace
	$(CARGO) build --workspace

run: ## cargo run (tradar binary)
	$(CARGO) run

fmt: ## Apply cargo fmt to the whole workspace
	$(CARGO) fmt --all

fmt-check: ## Check formatting without writing changes
	$(CARGO) fmt --all -- --check

clippy: ## Lint the whole workspace, warnings as errors
	$(CARGO) clippy --all-targets --workspace -- -D warnings

check: fmt-check clippy test-unit ## Fast pre-commit gate: fmt + clippy + tests that don't need Docker

test: ## Run every test in the workspace (needs Docker for postgres/redis/mongo/elasticsearch)
	$(CARGO) test --workspace

test-unit: ## Tests that never touch Docker: core, connector-spi, query-workbench, sqlite, app
	$(CARGO) test --workspace \
		--exclude tradar-connector-postgres \
		--exclude tradar-connector-redis \
		--exclude tradar-connector-mongo \
		--exclude tradar-connector-elasticsearch \
		--exclude tradar-connector-cassandra \
		--exclude tradar-connector-rabbitmq \
		--exclude tradar-connector-kafka \
		--exclude tradar-connector-http

test-docker: test-postgres test-redis test-mongo test-elasticsearch test-cassandra test-rabbitmq test-kafka test-http ## Every connector whose tests need a Docker daemon (testcontainers)

test-core: ## tradar-core only (keymap, storage, theme, config, ui, vim_list)
	$(CARGO) test -p tradar-core --lib

test-connector-spi: ## tradar-connector-spi only (CONNECT_TIMEOUT etc.)
	$(CARGO) test -p tradar-connector-spi --lib

test-workbench: ## tradar-query-workbench only (query editor, results, engine -- no Docker)
	$(CARGO) test -p tradar-query-workbench --lib

test-app: ## tradar-app only (components, RootComponent -- no Docker)
	$(CARGO) test -p tradar-app --lib

test-sqlite: ## tradar-connector-sqlite only (real temp-file DB, no Docker)
	$(CARGO) test -p tradar-connector-sqlite --lib

test-postgres: ## tradar-connector-postgres only -- needs Docker (testcontainers)
	$(CARGO) test -p tradar-connector-postgres --lib

test-redis: ## tradar-connector-redis only -- needs Docker (testcontainers)
	$(CARGO) test -p tradar-connector-redis --lib

test-mongo: ## tradar-connector-mongo only -- needs Docker (testcontainers)
	$(CARGO) test -p tradar-connector-mongo --lib

test-elasticsearch: ## tradar-connector-elasticsearch only -- needs Docker (testcontainers)
	$(CARGO) test -p tradar-connector-elasticsearch --lib

test-cassandra: ## tradar-connector-cassandra only -- needs Docker (testcontainers, GenericImage since no testcontainers-modules support)
	$(CARGO) test -p tradar-connector-cassandra --lib

test-rabbitmq: ## tradar-connector-rabbitmq only -- needs Docker (testcontainers, GenericImage since no testcontainers-modules support)
	$(CARGO) test -p tradar-connector-rabbitmq --lib

test-kafka: ## tradar-connector-kafka only -- needs Docker (testcontainers, GenericImage since no testcontainers-modules support)
	$(CARGO) test -p tradar-connector-kafka --lib

test-http: ## tradar-connector-http only -- needs Docker (testcontainers, GenericImage; no long-lived compose service since it targets arbitrary URLs, not one fixed backend)
	$(CARGO) test -p tradar-connector-http --lib

# --- docker-compose (docker-compose.yml) ---
# These are long-lived dev instances for manually running `tradar` against,
# separate from `cargo test`: the testcontainers-based tests above (postgres/
# redis/mongo/elasticsearch) spin up and tear down their own throwaway
# containers regardless of whether anything here is running.
# Valid names: postgres redis mongo elasticsearch5 elasticsearch7
# elasticsearch8 clickhouse cassandra rabbitmq kafka

up: ## Start every docker-compose service
	$(COMPOSE) up -d

down: ## Stop and remove every docker-compose service
	$(COMPOSE) down

ps: ## Show docker-compose service status
	$(COMPOSE) ps

up-%: ## Start one service, e.g. `make up-postgres` -- see the service list above
	$(COMPOSE) up -d $*

down-%: ## Stop and remove one service, e.g. `make down-postgres`
	$(COMPOSE) down $*
