// Rendering logic for HawkEye TUI panels

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table},
};

use crate::app::{AlertSeverity, HawkEyeApp, HawkEyeView, OrchestrationNodeStatus, format_bytes, format_uptime, state_label};

pub fn draw(f: &mut Frame, app: &HawkEyeApp) {
    match &app.view {
        HawkEyeView::Dashboard => draw_dashboard(f, app),
        HawkEyeView::AgentDetail(pid) => draw_agent_detail(f, app, *pid),
        HawkEyeView::AlertsPanel => draw_alerts_panel(f, app),
        HawkEyeView::OrchestrationGraph => draw_orchestration_graph(f, app),
    }
}

fn draw_dashboard(f: &mut Frame, app: &HawkEyeApp) {
    let area = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(8), Constraint::Length(1)])
        .split(area);
    draw_agent_list(f, app, chunks[0]);
    draw_alerts_summary(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
}

fn draw_agent_list(f: &mut Frame, app: &HawkEyeApp, area: Rect) {
    let filtered = app.filtered_agents();
    let header = Row::new(vec![
        Cell::from("PID").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("NAME").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("STATE").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("UPTIME").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("CPU%").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from("MEM").style(Style::default().add_modifier(Modifier::BOLD)),
    ]);
    let rows: Vec<Row> = filtered.iter().enumerate().map(|(i, a)| {
        let style = if app.selected_agent == Some(i) {
            Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        Row::new(vec![
            Cell::from(a.pid.to_string()),
            Cell::from(a.name.clone()),
            Cell::from(state_label(&a.state)),
            Cell::from(format_uptime(a.uptime)),
            Cell::from(format!("{:.1}", a.cpu_percent)),
            Cell::from(format_bytes(a.memory_bytes)),
        ]).style(style)
    }).collect();

    let title = if app.search_query.is_empty() {
        "Agents".to_string()
    } else {
        format!("Agents [filter: {}]", app.search_query)
    };

    let table = Table::new(rows, [
        Constraint::Length(8),
        Constraint::Min(16),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(6),
        Constraint::Length(10),
    ])
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(title));
    f.render_widget(table, area);
}

fn draw_alerts_summary(f: &mut Frame, app: &HawkEyeApp, area: Rect) {
    let sorted = app.sorted_alerts();
    let items: Vec<ListItem> = sorted.iter().take(5).map(|a| {
        let color = match a.severity {
            AlertSeverity::Error => Color::Red,
            AlertSeverity::Warning => Color::Yellow,
            AlertSeverity::Info => Color::Cyan,
        };
        let line = Line::from(vec![
            Span::styled(format!("[{}] ", a.timestamp), Style::default().fg(Color::DarkGray)),
            Span::styled(a.message.clone(), Style::default().fg(color)),
        ]);
        ListItem::new(line)
    }).collect();
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Alerts (recent)"));
    f.render_widget(list, area);
}

fn draw_status_bar(f: &mut Frame, app: &HawkEyeApp, area: Rect) {
    let msg = app.status_message.as_deref()
        .unwrap_or("q:quit  j/k:nav  Enter:detail  u:undo  /:search  Tab:panel");
    let para = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
    f.render_widget(para, area);
}

fn draw_agent_detail(f: &mut Frame, app: &HawkEyeApp, pid: u32) {
    let area = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(1)])
        .split(area);

    let content = match app.agents.iter().find(|a| a.pid == pid) {
        None => format!("Agent {pid} not found."),
        Some(a) => format!(
            "PID:    {}\nName:   {}\nState:  {}\nUptime: {}\nCPU:    {:.1}%\nMemory: {}\nFDs:    {}",
            a.pid, a.name, state_label(&a.state), format_uptime(a.uptime),
            a.cpu_percent, format_bytes(a.memory_bytes), a.open_fds,
        ),
    };

    let para = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title(format!("Agent Detail — PID {pid}")));
    f.render_widget(para, chunks[0]);
    draw_status_bar(f, app, chunks[1]);
}

fn draw_alerts_panel(f: &mut Frame, app: &HawkEyeApp) {
    let area = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    let sorted = app.sorted_alerts();
    let items: Vec<ListItem> = sorted.iter().map(|a| {
        let color = match a.severity {
            AlertSeverity::Error => Color::Red,
            AlertSeverity::Warning => Color::Yellow,
            AlertSeverity::Info => Color::Cyan,
        };
        let severity_label = match a.severity {
            AlertSeverity::Error => "ERROR",
            AlertSeverity::Warning => "WARN ",
            AlertSeverity::Info => "INFO ",
        };
        let line = Line::from(vec![
            Span::styled(format!("[{}] ", a.timestamp), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{severity_label} "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::raw(a.message.clone()),
        ]);
        ListItem::new(line)
    }).collect();

    let mut state = ListState::default();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("All Alerts"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    f.render_stateful_widget(list, chunks[0], &mut state);
    draw_status_bar(f, app, chunks[1]);
}

fn draw_orchestration_graph(f: &mut Frame, app: &HawkEyeApp) {
    let area = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(area);

    let items: Vec<ListItem> = if app.orchestration_nodes.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No orchestration plan active.",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.orchestration_nodes.iter().map(|node| {
            let (color, status_label) = match &node.status {
                OrchestrationNodeStatus::Pending => (Color::DarkGray, "Pending"),
                OrchestrationNodeStatus::Running => (Color::Yellow, "Running"),
                OrchestrationNodeStatus::Completed => (Color::Green, "Completed"),
                OrchestrationNodeStatus::Failed(_) => (Color::Red, "Failed"),
            };
            let agent_str = node.assigned_agent
                .map(|p| format!("agent:{p}"))
                .unwrap_or_else(|| "unassigned".to_string());
            let deps_str = if node.depends_on.is_empty() {
                String::new()
            } else {
                let deps: Vec<String> = node.depends_on.iter().map(|i| i.to_string()).collect();
                format!(" (after: {})", deps.join(", "))
            };
            let line = Line::from(vec![
                Span::styled(format!("[{}] ", node.index), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{:<12}", status_label), Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::raw(format!(" {} — {}{}", node.description, agent_str, deps_str)),
            ]);
            ListItem::new(line)
        }).collect()
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Orchestration Graph"));
    f.render_widget(list, chunks[0]);
    draw_status_bar(f, app, chunks[1]);
}
