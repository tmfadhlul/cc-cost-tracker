package handlers

import (
	"encoding/json"
	"fmt"
	"html/template"
	"log"
	"net/http"
	"path/filepath"
	"strings"
	"time"
)

// ── API response types ────────────────────────────────────────────────────────

type CostSummary struct {
	Cost             float64 `json:"cost"`
	InputTokens      uint64  `json:"input_tokens"`
	OutputTokens     uint64  `json:"output_tokens"`
	CacheWriteTokens uint64  `json:"cache_write_tokens"`
	CacheReadTokens  uint64  `json:"cache_read_tokens"`
}

type DailySpend struct {
	Date string  `json:"date"`
	Cost float64 `json:"cost"`
}

type CostBreakdown struct {
	Input      float64 `json:"input"`
	Output     float64 `json:"output"`
	CacheRead  float64 `json:"cache_read"`
	CacheWrite float64 `json:"cache_write"`
}

type ModelBreakdown struct {
	Model      string  `json:"model"`
	Cost       float64 `json:"cost"`
	Sessions   int     `json:"sessions"`
	PctOfTotal float64 `json:"pct_of_total"`
}

type HeatmapCell struct {
	Date     string             `json:"date"`
	Cost     float64            `json:"cost"`
	Projects map[string]float64 `json:"projects"`
}

type SessionSummary struct {
	ID          string  `json:"id"`
	Project     string  `json:"project"`
	Model       string  `json:"model"`
	LastActive  string  `json:"last_active"`
	TotalTokens uint64  `json:"total_tokens"`
	Cost        float64 `json:"cost"`
}

type ModelSeries struct {
	Model  string    `json:"model"`
	Daily  []float64 `json:"daily"`
	Hourly []float64 `json:"hourly"`
}

type OverviewData struct {
	Today           CostSummary      `json:"today"`
	Week            CostSummary      `json:"week"`
	Month           CostSummary      `json:"month"`
	Projected       CostSummary      `json:"projected"`
	WeekStartLabel  string           `json:"week_start_label"`
	MonthStartLabel string           `json:"month_start_label"`
	DailySpend      []DailySpend     `json:"daily_spend"`
	HourlySpend     []float64        `json:"hourly_spend"`
	HourlyLabels    []string         `json:"hourly_labels"`
	ModelSeries     []ModelSeries    `json:"model_series"`
	CostBreakdown   CostBreakdown    `json:"cost_breakdown"`
	ModelBreakdown  []ModelBreakdown `json:"model_breakdown"`
	ActivityHeatmap []HeatmapCell    `json:"activity_heatmap"`
	RecentSessions  []SessionSummary `json:"recent_sessions"`
}

type ProjectSummary struct {
	Name      string   `json:"name"`
	TotalCost float64  `json:"total_cost"`
	Sessions  int      `json:"sessions"`
	Models    []string `json:"models"`
	Subprojects []SubprojectSummary `json:"subprojects"`
}

type SubprojectSummary struct {
	Name      string   `json:"name"`
	TotalCost float64  `json:"total_cost"`
	Sessions  int      `json:"sessions"`
	Models    []string `json:"models"`
}

type RateEntry struct {
	Model             string  `json:"model"`
	InputPerMtok      float64 `json:"input_per_mtok"`
	OutputPerMtok     float64 `json:"output_per_mtok"`
	CacheWritePerMtok float64 `json:"cache_write_per_mtok"`
	CacheReadPerMtok  float64 `json:"cache_read_per_mtok"`
}

// ── Handler ───────────────────────────────────────────────────────────────────

type Handler struct {
	templateDir string
	backendURL  string
	client      *http.Client
}

func New(templateDir, backendURL string) *Handler {
	return &Handler{
		templateDir: templateDir,
		backendURL:  backendURL,
		client:      &http.Client{Timeout: 5 * time.Second},
	}
}

func (h *Handler) fetchJSON(path string, out any) error {
	resp, err := h.client.Get(h.backendURL + path)
	if err != nil {
		return fmt.Errorf("fetch %s: %w", path, err)
	}
	defer resp.Body.Close()
	return json.NewDecoder(resp.Body).Decode(out)
}

func (h *Handler) render(w http.ResponseWriter, name string, data any) {
	layout := filepath.Join(h.templateDir, "layout.html")
	page := filepath.Join(h.templateDir, name)
	tmpl, err := template.New("layout.html").Funcs(tmplFuncs()).ParseFiles(layout, page)
	if err != nil {
		log.Printf("template parse error (%s): %v", name, err)
		http.Error(w, "Template error", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := tmpl.ExecuteTemplate(w, "layout", data); err != nil {
		log.Printf("template exec error (%s): %v", name, err)
	}
}

func tmplFuncs() template.FuncMap {
	return template.FuncMap{
		"fmtCost": func(f float64) string {
			if f >= 100 {
				return fmt.Sprintf("$%.2f", f)
			}
			if f >= 1 {
				return fmt.Sprintf("$%.2f", f)
			}
			return fmt.Sprintf("$%.4f", f)
		},
		"fmtTokens": func(n uint64) string {
			switch {
			case n >= 1_000_000_000:
				return fmt.Sprintf("%.1fB", float64(n)/1_000_000_000)
			case n >= 1_000_000:
				return fmt.Sprintf("%.1fM", float64(n)/1_000_000)
			case n >= 1_000:
				return fmt.Sprintf("%.1fK", float64(n)/1_000)
			default:
				return fmt.Sprintf("%d", n)
			}
		},
		"fmtPct": func(f float64) string {
			return fmt.Sprintf("%.1f%%", f)
		},
		"today": func() string {
			return time.Now().Format("Monday, 02 January 2006")
		},
		"shortID": func(s string) string {
			if len(s) > 8 {
				return s[:8]
			}
			return s
		},
		"shortModel": func(s string) string {
			s = strings.TrimPrefix(s, "claude-")
			return s
		},
		"fmtDate": func(s string) string {
			t, err := time.Parse(time.RFC3339, s)
			if err != nil {
				return s
			}
			return t.Format("Jan 02 15:04")
		},
		"jsonMarshal": func(v any) (template.JS, error) {
			b, err := json.Marshal(v)
			return template.JS(b), err
		},
		"dailyLabels": func(items []DailySpend) []string {
			out := make([]string, len(items))
			for i, d := range items {
				t, _ := time.Parse("2006-01-02", d.Date)
				out[i] = t.Format("Jan 2")
			}
			return out
		},
		"dailyValues": func(items []DailySpend) []float64 {
			out := make([]float64, len(items))
			for i, d := range items {
				out[i] = d.Cost
			}
			return out
		},
		"hourLabels": func() []string {
			labels := make([]string, 24)
			for i := range labels {
				labels[i] = fmt.Sprintf("%02d:00", i)
			}
			return labels
		},
		"breakdownValues": func(b CostBreakdown) []float64 {
			return []float64{b.Input, b.Output, b.CacheRead, b.CacheWrite}
		},
		"joinStrings": func(ss []string) string {
			return strings.Join(ss, ", ")
		},
	}
}

// ── Page structs ──────────────────────────────────────────────────────────────

type basePage struct {
	Title  string
	Active string
	WSURL  string
}

type overviewPage struct {
	basePage
	Data OverviewData
}

type sessionsPage struct {
	basePage
	Sessions []SessionSummary
}

type projectsPage struct {
	basePage
	Projects []ProjectSummary
}

type rateCardPage struct {
	basePage
	Entries []RateEntry
}

// ── Handlers ──────────────────────────────────────────────────────────────────

func (h *Handler) Overview(w http.ResponseWriter, r *http.Request) {
	if r.URL.Path != "/" {
		http.NotFound(w, r)
		return
	}
	var data OverviewData
	if err := h.fetchJSON("/api/overview", &data); err != nil {
		log.Printf("overview: %v", err)
	}
	h.render(w, "overview.html", overviewPage{
		basePage: basePage{Title: "Overview", Active: "overview", WSURL: "ws://localhost:8080/ws"},
		Data:     data,
	})
}

func (h *Handler) Sessions(w http.ResponseWriter, r *http.Request) {
	var sessions []SessionSummary
	if err := h.fetchJSON("/api/sessions", &sessions); err != nil {
		log.Printf("sessions: %v", err)
	}
	h.render(w, "sessions.html", sessionsPage{
		basePage: basePage{Title: "Sessions", Active: "sessions"},
		Sessions: sessions,
	})
}

func (h *Handler) Projects(w http.ResponseWriter, r *http.Request) {
	var projects []ProjectSummary
	if err := h.fetchJSON("/api/projects", &projects); err != nil {
		log.Printf("projects: %v", err)
	}
	h.render(w, "projects.html", projectsPage{
		basePage: basePage{Title: "Projects", Active: "projects"},
		Projects: projects,
	})
}

func (h *Handler) Settings(w http.ResponseWriter, r *http.Request) {
	h.render(w, "settings.html", basePage{Title: "Settings", Active: "settings"})
}

func (h *Handler) RateCard(w http.ResponseWriter, r *http.Request) {
	var entries []RateEntry
	if err := h.fetchJSON("/api/rate-card", &entries); err != nil {
		log.Printf("rate-card: %v", err)
	}
	h.render(w, "rate-card.html", rateCardPage{
		basePage: basePage{Title: "Rate Card", Active: "rate-card"},
		Entries:  entries,
	})
}
