use crate::models::*;
use chrono::{Datelike, Duration, Local, TimeZone, Timelike, Utc};
use std::collections::{HashMap, HashSet};

pub fn build_overview(state: &AppState) -> OverviewResponse {
    let now = Utc::now();
    // Use local midnight for period boundaries so "today/week/month"
    // match the user's clock, not UTC.
    let now_local = now.with_timezone(&Local);
    let local_date = now_local.date_naive();

    let today_start = Local
        .from_local_datetime(&local_date.and_hms_opt(0, 0, 0).unwrap())
        .unwrap()
        .with_timezone(&Utc);
    let week_start = Local
        .from_local_datetime(
            &(local_date - Duration::days(now_local.weekday().num_days_from_sunday() as i64))
                .and_hms_opt(0, 0, 0)
                .unwrap(),
        )
        .unwrap()
        .with_timezone(&Utc);
    let month_start = Local
        .from_local_datetime(
            &local_date.with_day(1).unwrap().and_hms_opt(0, 0, 0).unwrap(),
        )
        .unwrap()
        .with_timezone(&Utc);

    let mut today  = CostSummary::default();
    let mut week   = CostSummary::default();
    let mut month  = CostSummary::default();
    let mut breakdown = CostBreakdown::default();
    let heatmap_window_start = now - Duration::days(365);
    let mut heatmap: HashMap<String, (f64, HashMap<String, f64>)> = HashMap::new();
    let mut model_cost: HashMap<String, f64> = HashMap::new();
    let mut model_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    let mut daily_map: HashMap<String, f64> = HashMap::new();
    let mut hourly = vec![0.0f64; 24];
    // Rolling 24h window: bucket 0 = oldest hour, bucket 23 = most recent
    let hourly_window_start = now - Duration::hours(24);
    // per-model: model → date → cost
    let mut model_daily: HashMap<String, HashMap<String, f64>> = HashMap::new();
    // per-model: model → 24 hourly values
    let mut model_hourly: HashMap<String, Vec<f64>> = HashMap::new();

    for r in &state.records {
        let cost = r.total_cost;
        let local_ts = r.timestamp.with_timezone(&Local);

        if r.timestamp >= today_start {
            accum_summary(&mut today, r);
        }
        if r.timestamp >= week_start {
            accum_summary(&mut week, r);
        }
        if r.timestamp >= month_start {
            accum_summary(&mut month, r);
        }

        breakdown.input       += r.cost_input;
        breakdown.output      += r.cost_output;
        breakdown.cache_read  += r.cost_cache_read;
        breakdown.cache_write += r.cost_cache_write;

        let hour = local_ts.hour();
        let date = local_ts.format("%Y-%m-%d").to_string();
        *daily_map.entry(date.clone()).or_default() += cost;
        if r.timestamp >= heatmap_window_start {
            let he = heatmap.entry(date.clone()).or_insert_with(|| (0.0, HashMap::new()));
            he.0 += cost;
            *he.1.entry(r.project.clone()).or_default() += cost;
        }

        *model_cost.entry(r.model.clone()).or_default() += cost;
        model_sessions.entry(r.model.clone()).or_default().insert(r.session_id.clone());

        // per-model daily
        *model_daily.entry(r.model.clone()).or_default().entry(date).or_default() += cost;

        // hourly buckets — rolling last 24h, bucket = position in window
        if r.timestamp >= hourly_window_start {
            let bucket = ((r.timestamp - hourly_window_start).num_seconds() / 3600)
                .min(23).max(0) as usize;
            hourly[bucket] += cost;
            let mh = model_hourly.entry(r.model.clone()).or_insert_with(|| vec![0.0; 24]);
            mh[bucket] += cost;
        }
    }

    // Daily spend — last 14 days
    let dates_14: Vec<String> = (0..14i64)
        .rev()
        .map(|i| (now_local - Duration::days(i)).format("%Y-%m-%d").to_string())
        .collect();

    let daily_spend: Vec<DailySpend> = dates_14
        .iter()
        .map(|date| DailySpend { date: date.clone(), cost: *daily_map.get(date).unwrap_or(&0.0) })
        .collect();

    // Model series (sorted by all-time cost desc — opus first)
    let mut model_series: Vec<ModelSeries> = model_daily
        .iter()
        .map(|(model, date_map)| {
            let daily = dates_14.iter().map(|d| *date_map.get(d).unwrap_or(&0.0)).collect();
            let hourly = model_hourly.get(model).cloned().unwrap_or_else(|| vec![0.0; 24]);
            ModelSeries { model: model.clone(), daily, hourly }
        })
        .collect();
    model_series.sort_by(|a, b| {
        let ca: f64 = a.daily.iter().sum::<f64>() + a.hourly.iter().sum::<f64>();
        let cb: f64 = b.daily.iter().sum::<f64>() + b.hourly.iter().sum::<f64>();
        cb.partial_cmp(&ca).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Model breakdown
    let total_cost: f64 = model_cost.values().sum();
    let mut model_breakdown: Vec<ModelBreakdown> = model_cost
        .iter()
        .map(|(model, &cost)| {
            let sessions = model_sessions.get(model).map_or(0, |s| s.len());
            let pct_of_total = if total_cost > 0.0 { cost / total_cost * 100.0 } else { 0.0 };
            ModelBreakdown { model: model.clone(), cost, sessions, pct_of_total }
        })
        .collect();
    model_breakdown.sort_by(|a, b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));

    // Heatmap (last 30 days, one cell per local date)
    let activity_heatmap: Vec<HeatmapCell> = heatmap
        .into_iter()
        .map(|(date, (cost, projects))| HeatmapCell { date, cost, projects })
        .collect();

    // Projected month cost
    let elapsed_secs = (now - month_start).num_seconds() as f64;
    let days_in_month = days_in_month(now.year(), now.month()) as f64;
    let projected_cost = if elapsed_secs > 0.0 {
        (month.cost / elapsed_secs) * days_in_month * 86400.0
    } else {
        0.0
    };

    // Recent sessions (top 20 by last_active)
    let recent_sessions = build_sessions_inner(state, 20);

    OverviewResponse {
        today,
        week,
        month,
        projected: CostSummary { cost: projected_cost, ..Default::default() },
        week_start_label: week_start.with_timezone(&Local).format("%b %-d").to_string(),
        month_start_label: month_start.with_timezone(&Local).format("%b %-d").to_string(),
        daily_spend,
        hourly_spend: hourly.clone(),
        hourly_labels: (0..24u32)
            .map(|i| {
                let t = (hourly_window_start + Duration::hours(i as i64))
                    .with_timezone(&Local);
                format!("{} {} {:02}:00", t.format("%b"), t.day(), t.hour())
            })
            .collect(),
        model_series,
        cost_breakdown: breakdown,
        model_breakdown,
        activity_heatmap,
        recent_sessions,
    }
}

fn accum_summary(s: &mut CostSummary, r: &UsageRecord) {
    s.cost               += r.total_cost;
    s.input_tokens       += r.input_tokens;
    s.output_tokens      += r.output_tokens;
    s.cache_write_tokens += r.cache_write_tokens;
    s.cache_read_tokens  += r.cache_read_tokens;
}

pub fn build_sessions(state: &AppState) -> Vec<SessionSummary> {
    build_sessions_inner(state, usize::MAX)
}

fn build_sessions_inner(state: &AppState, limit: usize) -> Vec<SessionSummary> {
    let mut map: HashMap<String, SessionSummary> = HashMap::new();

    for r in &state.records {
        let e = map.entry(r.session_id.clone()).or_insert_with(|| SessionSummary {
            id: r.session_id.clone(),
            project: r.project.clone(),
            model: r.model.clone(),
            last_active: r.timestamp.to_rfc3339(),
            total_tokens: 0,
            cost: 0.0,
        });
        e.total_tokens += r.input_tokens + r.output_tokens + r.cache_write_tokens + r.cache_read_tokens;
        e.cost += r.total_cost;
        if r.timestamp.to_rfc3339() > e.last_active {
            e.last_active = r.timestamp.to_rfc3339();
            e.model = r.model.clone();
        }
    }

    let mut sessions: Vec<SessionSummary> = map.into_values().collect();
    sessions.sort_by(|a, b| b.last_active.cmp(&a.last_active));
    sessions.truncate(limit);
    sessions
}

pub fn build_projects(state: &AppState) -> Vec<ProjectSummary> {
    let mut map: HashMap<String, (f64, HashSet<String>, HashSet<String>, HashMap<String, (f64, HashSet<String>, HashSet<String>)>)> = HashMap::new();

    for r in &state.records {
        let e = map.entry(r.project.clone()).or_default();
        e.0 += r.total_cost;
        e.1.insert(r.session_id.clone());
        e.2.insert(r.model.clone());

        if !r.subprojects.is_empty() {
            let share = r.total_cost / r.subprojects.len() as f64;
            for subproject in &r.subprojects {
                let sub = e.3.entry(subproject.clone()).or_default();
                sub.0 += share;
                sub.1.insert(r.session_id.clone());
                sub.2.insert(r.model.clone());
            }
        }
    }

    let mut projects: Vec<ProjectSummary> = map
        .into_iter()
        .map(|(name, (total_cost, sessions, models, subprojects))| {
            let mut subprojects: Vec<SubprojectSummary> = subprojects
                .into_iter()
                .map(|(name, (total_cost, sessions, models))| SubprojectSummary {
                    name,
                    total_cost,
                    sessions: sessions.len(),
                    models: models.into_iter().collect(),
                })
                .collect();
            subprojects.sort_by(|a, b| b.total_cost.partial_cmp(&a.total_cost).unwrap_or(std::cmp::Ordering::Equal));

            ProjectSummary {
                name,
                total_cost,
                sessions: sessions.len(),
                models: models.into_iter().collect(),
                subprojects,
            }
        })
        .collect();
    projects.sort_by(|a, b| b.total_cost.partial_cmp(&a.total_cost).unwrap_or(std::cmp::Ordering::Equal));
    projects
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let next_month_first = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
    };
    let this_month_first = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (next_month_first - this_month_first).num_days() as u32
}
