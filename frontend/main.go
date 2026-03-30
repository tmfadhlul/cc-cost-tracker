package main

import (
	"log"
	"net/http"
	"os"
	"path/filepath"

	"cc-cost-frontend/handlers"
)

func main() {
	templateDir := findTemplateDir()
	backendURL := envOr("BACKEND_URL", "http://localhost:8080")

	h := handlers.New(templateDir, backendURL)

	staticDir := findStaticDir()

	mux := http.NewServeMux()
	mux.Handle("/static/", http.StripPrefix("/static/", http.FileServer(http.Dir(staticDir))))
	mux.HandleFunc("/", h.Overview)
	mux.HandleFunc("/sessions", h.Sessions)
	mux.HandleFunc("/projects", h.Projects)
	mux.HandleFunc("/settings", h.Settings)
	mux.HandleFunc("/rate-card", h.RateCard)

	addr := envOr("FRONTEND_ADDR", ":3000")
	log.Printf("Frontend listening on http://localhost%s", addr)
	if err := http.ListenAndServe(addr, mux); err != nil {
		log.Fatal(err)
	}
}

func findStaticDir() string {
	if _, err := os.Stat("static"); err == nil {
		return "static"
	}
	exe, err := os.Executable()
	if err == nil {
		d := filepath.Join(filepath.Dir(exe), "static")
		if _, err := os.Stat(d); err == nil {
			return d
		}
	}
	return "static"
}

func findTemplateDir() string {
	// When running via `go run .` the CWD contains templates/
	if _, err := os.Stat("templates"); err == nil {
		return "templates"
	}
	// When running a compiled binary, look next to the executable
	exe, err := os.Executable()
	if err == nil {
		d := filepath.Join(filepath.Dir(exe), "templates")
		if _, err := os.Stat(d); err == nil {
			return d
		}
	}
	return "templates"
}

func envOr(key, fallback string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return fallback
}
