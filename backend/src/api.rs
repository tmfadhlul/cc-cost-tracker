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
    let mut heatmap: HashMap<(u32, u32), f64> = HashMap::new();
    let mut model_cost: HashMap<String, f64> = HashMap::new();
    let mut model_sessions: HashMap<String, HashSet<String>> = HashMap::new();
    let mut daily_map: HashMap<String, f64> = HashMap::new();
    let mut hourly = vec![0.0f64; 24];

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
        let dow  = local_ts.weekday().num_days_from_sunday();
        *heatmap.entry((hour, dow)).or_default() += cost;
        hourly[hour as usize] += cost;

        let date = r.timestamp.format("%Y-%m-%d").to_string();
        *daily_map.entry(date).or_default() += cost;

        *model_cost.entry(r.model.clone()).or_default() += cost;
        model_sessions.entry(r.model.clone()).or_default().insert(r.session_id.clone());
    }

    // Daily spend — last 14 days
    let daily_spend: Vec<DailySpend> = (0..14i64)
        .rev()
        .map(|i| {
            let date = (now - Duration::days(i)).format("%Y-%m-%d").to_string();
            let cost = *daily_map.get(&date).unwrap_or(&0.0);
            DailySpend { date, cost }
        })
        .collect();

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

    // Heatmap
    let activity_heatmap: Vec<HeatmapCell> = heatmap
        .into_iter()
        .map(|((hour, day_of_week), cost)| HeatmapCell { hour, day_of_week, cost })
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
        daily_spend,
        hourly_spend: hourly,
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
    let mut map: HashMap<String, (f64, HashSet<String>, HashSet<String>)> = HashMap::new();

    for r in &state.records {
        let e = map.entry(r.project.clone()).or_default();
        e.0 += r.total_cost;
        e.1.insert(r.session_id.clone());
        e.2.insert(r.model.clone());
    }

    let mut projects: Vec<ProjectSummary> = map
        .into_iter()
        .map(|(name, (total_cost, sessions, models))| ProjectSummary {
            name,
            total_cost,
            sessions: sessions.len(),
            models: models.into_iter().collect(),
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
