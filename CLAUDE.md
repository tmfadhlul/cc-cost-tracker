# Project: cc-cost (Claude Code Cost Tracker)

## Obsidian Integration
This project is tracked in the Obsidian vault as `PRJ - cc-cost (Claude Code Cost Tracker)`.

When working on this project:
- Check `02-Projects/PRJ - cc-cost (Claude Code Cost Tracker).md` for context, decisions, and known gotchas
- Update that note when making significant changes, fixing bugs, or hitting new decisions
- Log entries go under the `## Log` section (newest first, date format `YYYY-MM-DD`)

## Tech Stack
- **Rust** (axum, notify, serde, chrono) — `backend/src/`
- **Go + HTMX** (net/http, html/template) — `frontend/`
- **Python** proxy for Copilot/Anthropic API tracking — `proxy/proxy.py`
- **Chart.js** — charts in `frontend/templates/overview.html`
- **launchd / systemd** — `make install-service`

## Key Rules
- The tracker reads `~/.claude/projects/**/*.jsonl` — never write to those files
- The tracker also reads `~/.cctrack/proxy/*.jsonl` for Copilot proxy logs
- Deduplication is GLOBAL across files (shared `seen` map in `parser.rs`) — do not scope it per-file
- Nested repo categorization is based on detecting nested `.git` roots under the workspace, then mapping touched files into those repos
- Period boundaries (today/week/month) use LOCAL timezone, not UTC — keep it that way
- Chart.js options must never be shared between chart instances — always call `stackedBarOpts()` to get a fresh object
- Costs are pay-as-you-go equivalent, not actual billing — make this clear in any UI copy

## Ports
- Backend: `8080`
- Frontend: `45123`
- Proxy: `4001`

## Logs (when running as service)
- `~/.cctrack/backend.log`
- `~/.cctrack/frontend.log`
