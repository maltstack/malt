use crate::client::SessionData;

pub fn print_sessions(sessions: &[SessionData]) {
    if sessions.is_empty() {
        println!("  no sessions");
        return;
    }
    println!(
        "  {:>4}  {:>5}  {:>10}  {:>8}  {}",
        "ID", "PANES", "ISOLATION", "STATE", "NAME"
    );
    for s in sessions {
        println!(
            "  {:>4}  {:>5}  {:>10}  {:>8}  {}",
            s.id,
            s.pane_count,
            s.isolation,
            s.state,
            s.name.as_deref().unwrap_or("-")
        );
    }
}
