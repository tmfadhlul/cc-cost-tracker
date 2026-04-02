use crate::models::{BroadcastTx, SharedState};
use crate::parser::{proxy_log_dir, scan_all_records};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

pub async fn start_watcher(state: SharedState, tx: BroadcastTx) {
    let home = std::env::var("HOME").expect("HOME not set");
    let watch_path = PathBuf::from(&home).join(".claude").join("projects");
    let proxy_path = proxy_log_dir();

    // Create proxy log dir if it doesn't exist so the watcher can attach
    let _ = std::fs::create_dir_all(&proxy_path);

    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<PathBuf>();

    let notify_tx2 = notify_tx.clone();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        for path in event.paths {
                            if path.extension().map_or(false, |e| e == "jsonl") {
                                let _ = notify_tx2.send(path);
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .expect("Failed to create watcher");

    watcher
        .watch(&watch_path, RecursiveMode::Recursive)
        .expect("Failed to watch ~/.claude/projects");

    watcher
        .watch(&proxy_path, RecursiveMode::Recursive)
        .unwrap_or_else(|e| tracing::warn!("Could not watch proxy log dir {:?}: {}", proxy_path, e));

    tracing::info!("Watching {:?}", watch_path);
    tracing::info!("Watching {:?}", proxy_path);

    // Keep watcher alive for the duration of this async task
    let _watcher = watcher;

    loop {
        // Wait for the first file event
        if notify_rx.recv().await.is_none() {
            break;
        }

        // Drain additional events within 300ms debounce window
        tokio::time::sleep(Duration::from_millis(300)).await;
        while notify_rx.try_recv().is_ok() {}

        tracing::info!("File change detected — rescanning logs...");

        let records = tokio::task::spawn_blocking(scan_all_records)
            .await
            .unwrap_or_default();

        let count = records.len();
        {
            let mut s = state.write().await;
            s.records = records;
        }
        tracing::info!("Loaded {} records", count);

        // Broadcast updated overview to all WebSocket clients
        let overview = {
            let s = state.read().await;
            crate::api::build_overview(&s)
        };
        if let Ok(json) = serde_json::to_string(&overview) {
            let _ = tx.send(json);
        }
    }
}
