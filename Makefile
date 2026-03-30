.PHONY: install install-rust install-go build build-backend build-frontend dev run run-backend run-frontend clean open

BACKEND_DIR  := backend
FRONTEND_DIR := frontend
BACKEND_BIN  := $(BACKEND_DIR)/target/release/cc-cost-backend
FRONTEND_BIN := $(FRONTEND_DIR)/cc-cost-frontend

# Detect OS
UNAME := $(shell uname -s)

# ── Install toolchains ────────────────────────────────────────────────────────

install: install-rust install-go
	@echo ""
	@echo "All toolchains ready. Run 'make dev' to start."

install-rust:
	@if command -v cargo &>/dev/null || [ -f "$$HOME/.cargo/bin/cargo" ]; then \
		echo "Rust already installed: $$(rustc --version 2>/dev/null || $$HOME/.cargo/bin/rustc --version)"; \
	else \
		echo "Installing Rust via rustup..."; \
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path; \
		echo "Rust installed. Run: source $$HOME/.cargo/env"; \
	fi

install-go:
	@if command -v go &>/dev/null; then \
		echo "Go already installed: $$(go version)"; \
	elif [ "$(UNAME)" = "Darwin" ]; then \
		BREW=$$(command -v brew || echo /opt/homebrew/bin/brew || echo /usr/local/bin/brew); \
		if [ -x "$$BREW" ]; then \
			echo "Installing Go via Homebrew..."; \
			$$BREW install go; \
		else \
			echo "Homebrew not found. Install Go from https://go.dev/dl/"; \
			exit 1; \
		fi; \
	elif [ "$(UNAME)" = "Linux" ]; then \
		if command -v apt-get &>/dev/null; then \
			echo "Installing Go via apt..."; \
			sudo apt-get update -qq && sudo apt-get install -y golang-go; \
		elif command -v pacman &>/dev/null; then \
			echo "Installing Go via pacman..."; \
			sudo pacman -S --noconfirm go; \
		elif command -v dnf &>/dev/null; then \
			echo "Installing Go via dnf..."; \
			sudo dnf install -y golang; \
		else \
			echo "Could not detect package manager. Install Go from https://go.dev/dl/"; \
			exit 1; \
		fi; \
	else \
		echo "Unsupported OS. Install Go from https://go.dev/dl/"; \
		exit 1; \
	fi

# ── Build ─────────────────────────────────────────────────────────────────────

build: build-backend build-frontend

build-backend:
	@echo "Building Rust backend..."
	@. "$$HOME/.cargo/env" 2>/dev/null || true; \
	cd $(BACKEND_DIR) && cargo build --release
	@echo "Backend built: $(BACKEND_BIN)"

build-frontend:
	@echo "Building Go frontend..."
	@cd $(FRONTEND_DIR) && go build -o cc-cost-frontend .
	@echo "Frontend built: $(FRONTEND_BIN)"

# ── Dev (fast iteration — cargo run + go run) ─────────────────────────────────

dev:
	@echo ""
	@echo "  Backend  → http://localhost:8080  (API + WebSocket)"
	@echo "  Frontend → http://localhost:3000  (Dashboard)"
	@echo ""
	@echo "Press Ctrl+C to stop."
	@echo ""
	@( \
	  . "$$HOME/.cargo/env" 2>/dev/null || true; \
	  trap 'kill 0' EXIT INT TERM; \
	  cd $(BACKEND_DIR) && cargo run 2>&1 | sed 's/^/[backend] /' & \
	  sleep 3 && cd $(FRONTEND_DIR) && go run . 2>&1 | sed 's/^/[frontend] /' & \
	  sleep 5 && $(MAKE) --no-print-directory open & \
	  wait \
	)

# ── Run release builds ────────────────────────────────────────────────────────

run: build
	@echo "Starting cc-cost (release builds)..."
	@( \
	  trap 'kill 0' EXIT INT TERM; \
	  $(BACKEND_BIN) 2>&1 | sed 's/^/[backend] /' & \
	  sleep 2 && $(FRONTEND_BIN) 2>&1 | sed 's/^/[frontend] /' & \
	  sleep 3 && $(MAKE) --no-print-directory open & \
	  wait \
	)

run-backend:
	@. "$$HOME/.cargo/env" 2>/dev/null || true; \
	cd $(BACKEND_DIR) && cargo run

run-frontend:
	@cd $(FRONTEND_DIR) && go run .

# ── Open browser ──────────────────────────────────────────────────────────────

open:
	@if [ "$(UNAME)" = "Darwin" ]; then \
		open http://localhost:3000; \
	elif command -v xdg-open &>/dev/null; then \
		xdg-open http://localhost:3000; \
	fi

# ── Clean ─────────────────────────────────────────────────────────────────────

clean:
	@cd $(BACKEND_DIR) && cargo clean 2>/dev/null || true
	@rm -f $(FRONTEND_BIN)
	@echo "Cleaned"
