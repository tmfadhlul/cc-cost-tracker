.PHONY: install install-rust install-go build build-backend build-frontend dev run run-backend run-frontend run-proxy clean open install-service uninstall-service service-status

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

run-proxy:
	@echo "Starting Anthropic API proxy on http://127.0.0.1:4001..."
	@python3 proxy/proxy.py

# ── Open browser ──────────────────────────────────────────────────────────────

open:
	@if [ "$(UNAME)" = "Darwin" ]; then \
		open http://localhost:3000; \
	elif command -v xdg-open &>/dev/null; then \
		xdg-open http://localhost:3000; \
	fi

# ── Service (macOS launchd / Linux systemd) ───────────────────────────────────

PROJECT_DIR := $(shell pwd)
BACKEND_ABS  := $(PROJECT_DIR)/$(BACKEND_BIN)
FRONTEND_ABS := $(PROJECT_DIR)/$(FRONTEND_BIN)
PLIST_DIR    := $(HOME)/Library/LaunchAgents
BACKEND_PLIST  := $(PLIST_DIR)/com.cctrack.backend.plist
FRONTEND_PLIST := $(PLIST_DIR)/com.cctrack.frontend.plist
SYSTEMD_DIR  := $(HOME)/.config/systemd/user

install-service: build
ifeq ($(UNAME), Darwin)
	@echo "Installing launchd services..."
	@mkdir -p $(PLIST_DIR) $(HOME)/.cctrack
	@printf '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n<dict>\n  <key>Label</key>             <string>com.cctrack.backend</string>\n  <key>ProgramArguments</key>  <array><string>%s</string></array>\n  <key>WorkingDirectory</key>  <string>%s</string>\n  <key>RunAtLoad</key>         <true/>\n  <key>KeepAlive</key>         <true/>\n  <key>StandardOutPath</key>   <string>%s/.cctrack/backend.log</string>\n  <key>StandardErrorPath</key> <string>%s/.cctrack/backend.log</string>\n</dict>\n</plist>\n' \
		"$(BACKEND_ABS)" "$(PROJECT_DIR)" "$(HOME)" "$(HOME)" > $(BACKEND_PLIST)
	@printf '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n<dict>\n  <key>Label</key>             <string>com.cctrack.frontend</string>\n  <key>ProgramArguments</key>  <array><string>%s</string></array>\n  <key>WorkingDirectory</key>  <string>%s</string>\n  <key>RunAtLoad</key>         <true/>\n  <key>KeepAlive</key>         <true/>\n  <key>StandardOutPath</key>   <string>%s/.cctrack/frontend.log</string>\n  <key>StandardErrorPath</key> <string>%s/.cctrack/frontend.log</string>\n</dict>\n</plist>\n' \
		"$(FRONTEND_ABS)" "$(PROJECT_DIR)" "$(HOME)" "$(HOME)" > $(FRONTEND_PLIST)
	@launchctl unload $(BACKEND_PLIST)  2>/dev/null || true
	@launchctl unload $(FRONTEND_PLIST) 2>/dev/null || true
	@lsof -ti :8080   2>/dev/null | xargs kill -9 2>/dev/null || true
	@lsof -ti :45123  2>/dev/null | xargs kill -9 2>/dev/null || true
	@launchctl load -w $(BACKEND_PLIST)
	@launchctl load -w $(FRONTEND_PLIST)
	@echo ""
	@echo "Services installed and running at startup."
	@echo "  Backend  → http://localhost:8080"
	@echo "  Frontend → http://localhost:45123"
	@echo ""
	@echo "Logs:  tail -f $(HOME)/.cctrack/backend.log"
	@echo "       tail -f $(HOME)/.cctrack/frontend.log"
else ifeq ($(UNAME), Linux)
	@echo "Installing systemd user services..."
	@mkdir -p $(SYSTEMD_DIR) $(HOME)/.cctrack
	@printf '[Unit]\nDescription=cctrack Rust backend\nAfter=network.target\n\n[Service]\nExecStart=%s\nWorkingDirectory=%s\nRestart=always\nRestartSec=3\nStandardOutput=append:%s/.cctrack/backend.log\nStandardError=append:%s/.cctrack/backend.log\n\n[Install]\nWantedBy=default.target\n' \
		"$(BACKEND_ABS)" "$(PROJECT_DIR)" "$(HOME)" "$(HOME)" > $(SYSTEMD_DIR)/cctrack-backend.service
	@printf '[Unit]\nDescription=cctrack Go frontend\nAfter=cctrack-backend.service\n\n[Service]\nExecStart=%s\nWorkingDirectory=%s\nRestart=always\nRestartSec=3\nStandardOutput=append:%s/.cctrack/frontend.log\nStandardError=append:%s/.cctrack/frontend.log\n\n[Install]\nWantedBy=default.target\n' \
		"$(FRONTEND_ABS)" "$(PROJECT_DIR)" "$(HOME)" "$(HOME)" > $(SYSTEMD_DIR)/cctrack-frontend.service
	@systemctl --user daemon-reload
	@systemctl --user stop cctrack-backend cctrack-frontend 2>/dev/null || true
	@lsof -ti :8080  2>/dev/null | xargs kill -9 2>/dev/null || true
	@lsof -ti :45123 2>/dev/null | xargs kill -9 2>/dev/null || true
	@systemctl --user enable --now cctrack-backend cctrack-frontend
	@echo ""
	@echo "Services installed and running at startup."
	@echo "  Backend  → http://localhost:8080"
	@echo "  Frontend → http://localhost:45123"
	@echo ""
	@echo "Logs:  journalctl --user -u cctrack-backend -f"
	@echo "       journalctl --user -u cctrack-frontend -f"
else
	@echo "Unsupported OS (macOS and Linux only)"
	@exit 1
endif

uninstall-service:
ifeq ($(UNAME), Darwin)
	@launchctl unload $(BACKEND_PLIST)  2>/dev/null && echo "Stopped backend"  || true
	@launchctl unload $(FRONTEND_PLIST) 2>/dev/null && echo "Stopped frontend" || true
	@rm -f $(BACKEND_PLIST) $(FRONTEND_PLIST)
	@echo "Services removed."
else ifeq ($(UNAME), Linux)
	@systemctl --user disable --now cctrack-backend cctrack-frontend 2>/dev/null || true
	@rm -f $(SYSTEMD_DIR)/cctrack-backend.service $(SYSTEMD_DIR)/cctrack-frontend.service
	@systemctl --user daemon-reload
	@echo "Services removed."
endif

service-status:
ifeq ($(UNAME), Darwin)
	@echo "=== Backend ===" && launchctl list | grep cctrack.backend  || echo "  not running"
	@echo "=== Frontend ===" && launchctl list | grep cctrack.frontend || echo "  not running"
else ifeq ($(UNAME), Linux)
	@systemctl --user status cctrack-backend cctrack-frontend --no-pager
endif

# ── Clean ─────────────────────────────────────────────────────────────────────

clean:
	@cd $(BACKEND_DIR) && cargo clean 2>/dev/null || true
	@rm -f $(FRONTEND_BIN)
	@echo "Cleaned"
