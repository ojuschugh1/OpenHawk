// Unit tests for hawk-ui (Tasks 16.4)

#[cfg(test)]
mod tests {
    use crossterm::event::KeyCode;
    use hawk_core::types::{AgentStatus, LifecycleState};
    use std::time::Duration;

    use crate::app::{Alert, AlertSeverity, AppAction, HawkEyeApp, HawkEyeView};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_agent(pid: u32, name: &str) -> AgentStatus {
        AgentStatus {
            pid,
            name: name.to_string(),
            state: LifecycleState::Running,
            uptime: Duration::from_secs(60),
            cpu_percent: 1.5,
            memory_bytes: 1024 * 1024,
            open_fds: 4,
        }
    }

    fn make_alert(ts: &str, msg: &str, sev: AlertSeverity) -> Alert {
        Alert {
            timestamp: ts.to_string(),
            message: msg.to_string(),
            severity: sev,
        }
    }

    fn app_with_agents(agents: Vec<AgentStatus>) -> HawkEyeApp {
        let mut app = HawkEyeApp::new();
        app.agents = agents;
        app
    }

    // ── Keyboard mapping (Task 16.1 / 16.4) ──────────────────────────────────

    #[test]
    fn key_q_maps_to_quit() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Char('q')), Some(AppAction::Quit));
    }

    #[test]
    fn key_j_maps_to_select_next() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Char('j')), Some(AppAction::SelectNext));
    }

    #[test]
    fn key_down_maps_to_select_next() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Down), Some(AppAction::SelectNext));
    }

    #[test]
    fn key_k_maps_to_select_prev() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Char('k')), Some(AppAction::SelectPrev));
    }

    #[test]
    fn key_up_maps_to_select_prev() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Up), Some(AppAction::SelectPrev));
    }

    #[test]
    fn key_enter_maps_to_open_detail() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Enter), Some(AppAction::OpenDetail));
    }

    #[test]
    fn key_esc_maps_to_close_detail() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Esc), Some(AppAction::CloseDetail));
    }

    #[test]
    fn key_slash_maps_to_start_search() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Char('/')), Some(AppAction::StartSearch));
    }

    #[test]
    fn key_tab_maps_to_switch_panel() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Tab), Some(AppAction::SwitchPanel));
    }

    #[test]
    fn key_u_maps_to_undo_selected() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Char('u')), Some(AppAction::UndoSelected));
    }

    #[test]
    fn unknown_key_returns_none() {
        let app = HawkEyeApp::new();
        assert_eq!(app.handle_key(KeyCode::Char('z')), None);
        assert_eq!(app.handle_key(KeyCode::F(1)), None);
    }

    // ── SelectNext / SelectPrev ───────────────────────────────────────────────

    #[test]
    fn select_next_from_none_selects_first() {
        let mut app = app_with_agents(vec![make_agent(1, "a"), make_agent(2, "b")]);
        app.apply_action(AppAction::SelectNext);
        assert_eq!(app.selected_agent, Some(0));
    }

    #[test]
    fn select_next_advances_index() {
        let mut app = app_with_agents(vec![make_agent(1, "a"), make_agent(2, "b")]);
        app.selected_agent = Some(0);
        app.apply_action(AppAction::SelectNext);
        assert_eq!(app.selected_agent, Some(1));
    }

    #[test]
    fn select_next_clamps_at_last() {
        let mut app = app_with_agents(vec![make_agent(1, "a"), make_agent(2, "b")]);
        app.selected_agent = Some(1);
        app.apply_action(AppAction::SelectNext);
        assert_eq!(app.selected_agent, Some(1));
    }

    #[test]
    fn select_prev_from_none_selects_first() {
        let mut app = app_with_agents(vec![make_agent(1, "a"), make_agent(2, "b")]);
        app.apply_action(AppAction::SelectPrev);
        assert_eq!(app.selected_agent, Some(0));
    }

    #[test]
    fn select_prev_decrements_index() {
        let mut app = app_with_agents(vec![make_agent(1, "a"), make_agent(2, "b")]);
        app.selected_agent = Some(1);
        app.apply_action(AppAction::SelectPrev);
        assert_eq!(app.selected_agent, Some(0));
    }

    #[test]
    fn select_prev_clamps_at_zero() {
        let mut app = app_with_agents(vec![make_agent(1, "a")]);
        app.selected_agent = Some(0);
        app.apply_action(AppAction::SelectPrev);
        assert_eq!(app.selected_agent, Some(0));
    }

    #[test]
    fn select_next_on_empty_list_is_noop() {
        let mut app = HawkEyeApp::new();
        app.apply_action(AppAction::SelectNext);
        assert_eq!(app.selected_agent, None);
    }

    // ── OpenDetail / CloseDetail ──────────────────────────────────────────────

    #[test]
    fn open_detail_switches_view_to_agent_detail() {
        let mut app = app_with_agents(vec![make_agent(42, "my-agent")]);
        app.selected_agent = Some(0);
        app.apply_action(AppAction::OpenDetail);
        assert_eq!(app.view, HawkEyeView::AgentDetail(42));
    }

    #[test]
    fn open_detail_with_no_selection_is_noop() {
        let mut app = app_with_agents(vec![make_agent(1, "a")]);
        app.apply_action(AppAction::OpenDetail);
        assert_eq!(app.view, HawkEyeView::Dashboard);
    }

    #[test]
    fn close_detail_returns_to_dashboard() {
        let mut app = HawkEyeApp::new();
        app.view = HawkEyeView::AgentDetail(99);
        app.apply_action(AppAction::CloseDetail);
        assert_eq!(app.view, HawkEyeView::Dashboard);
    }

    // ── SwitchPanel ───────────────────────────────────────────────────────────

    #[test]
    fn switch_panel_dashboard_to_alerts() {
        let mut app = HawkEyeApp::new();
        app.apply_action(AppAction::SwitchPanel);
        assert_eq!(app.view, HawkEyeView::AlertsPanel);
    }

    #[test]
    fn switch_panel_alerts_to_dashboard() {
        let mut app = HawkEyeApp::new();
        app.view = HawkEyeView::AlertsPanel;
        app.apply_action(AppAction::SwitchPanel);
        assert_eq!(app.view, HawkEyeView::Dashboard);
    }

    #[test]
    fn switch_panel_from_detail_goes_to_dashboard() {
        let mut app = HawkEyeApp::new();
        app.view = HawkEyeView::AgentDetail(1);
        app.apply_action(AppAction::SwitchPanel);
        assert_eq!(app.view, HawkEyeView::Dashboard);
    }

    // ── SetStatusMessage ──────────────────────────────────────────────────────

    #[test]
    fn set_status_message_stores_message() {
        let mut app = HawkEyeApp::new();
        app.apply_action(AppAction::SetStatusMessage("hello".to_string()));
        assert_eq!(app.status_message, Some("hello".to_string()));
    }

    // ── filtered_agents (Task 16.4) ───────────────────────────────────────────

    #[test]
    fn filtered_agents_empty_query_returns_all() {
        let app = app_with_agents(vec![
            make_agent(1, "alpha"),
            make_agent(2, "beta"),
            make_agent(3, "gamma"),
        ]);
        assert_eq!(app.filtered_agents().len(), 3);
    }

    #[test]
    fn filtered_agents_matches_substring_case_insensitive() {
        let mut app = app_with_agents(vec![
            make_agent(1, "research-agent"),
            make_agent(2, "coding-agent"),
            make_agent(3, "review-bot"),
        ]);
        app.search_query = "agent".to_string();
        let filtered = app.filtered_agents();
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().any(|a| a.pid == 1));
        assert!(filtered.iter().any(|a| a.pid == 2));
    }

    #[test]
    fn filtered_agents_no_match_returns_empty() {
        let mut app = app_with_agents(vec![make_agent(1, "alpha")]);
        app.search_query = "zzz".to_string();
        assert!(app.filtered_agents().is_empty());
    }

    #[test]
    fn filtered_agents_case_insensitive() {
        let mut app = app_with_agents(vec![make_agent(1, "MyAgent")]);
        app.search_query = "myagent".to_string();
        assert_eq!(app.filtered_agents().len(), 1);
    }

    // ── sorted_alerts (Task 16.4) ─────────────────────────────────────────────

    #[test]
    fn sorted_alerts_returns_most_recent_first() {
        let mut app = HawkEyeApp::new();
        app.alerts = vec![
            make_alert("2024-01-01T10:00:00Z", "old alert", AlertSeverity::Info),
            make_alert("2024-01-03T12:00:00Z", "newest alert", AlertSeverity::Error),
            make_alert("2024-01-02T08:00:00Z", "middle alert", AlertSeverity::Warning),
        ];
        let sorted = app.sorted_alerts();
        assert_eq!(sorted[0].message, "newest alert");
        assert_eq!(sorted[1].message, "middle alert");
        assert_eq!(sorted[2].message, "old alert");
    }

    #[test]
    fn sorted_alerts_empty_returns_empty() {
        let app = HawkEyeApp::new();
        assert!(app.sorted_alerts().is_empty());
    }

    #[test]
    fn sorted_alerts_single_element() {
        let mut app = HawkEyeApp::new();
        app.alerts = vec![make_alert("2024-06-01T00:00:00Z", "only", AlertSeverity::Info)];
        let sorted = app.sorted_alerts();
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0].message, "only");
    }

    // ── UndoSelected with no selection ────────────────────────────────────────

    #[test]
    fn undo_with_no_selection_sets_status_message() {
        let mut app = HawkEyeApp::new();
        app.apply_action(AppAction::UndoSelected);
        assert!(app.status_message.is_some());
        assert!(app.status_message.as_deref().unwrap().contains("No agent selected"));
    }

    // ── format helpers ────────────────────────────────────────────────────────

    #[test]
    fn format_uptime_zero() {
        use crate::app::format_uptime;
        assert_eq!(format_uptime(Duration::from_secs(0)), "00:00:00");
    }

    #[test]
    fn format_uptime_one_hour() {
        use crate::app::format_uptime;
        assert_eq!(format_uptime(Duration::from_secs(3661)), "01:01:01");
    }

    #[test]
    fn format_bytes_bytes() {
        use crate::app::format_bytes;
        assert_eq!(format_bytes(512), "512B");
    }

    #[test]
    fn format_bytes_kilobytes() {
        use crate::app::format_bytes;
        assert_eq!(format_bytes(2048), "2.0KB");
    }

    #[test]
    fn format_bytes_megabytes() {
        use crate::app::format_bytes;
        assert_eq!(format_bytes(1024 * 1024), "1.0MB");
    }
}
