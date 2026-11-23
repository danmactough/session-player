//! A report as Markdown. The same layout the app's "Copy as Markdown" writes
//! (`src/lib/reportMarkdown.ts`), with the actions numbered so `sp action`
//! can name one.

use std::collections::HashMap;

use chrono::TimeZone;
use session_player_core::annotations::{Kind, Outcome};
use session_player_core::reports::{
    ActionCategory, ActionResponse, ActionStatus, CitedAnnotation, Evidence, LatestVerdict, Priority, Report, UserImpact, Verdict,
};

pub fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::ToolError => "Tool error",
        Kind::HumanIntervention => "Human intervention",
        Kind::AgentStuck => "Agent stuck",
        Kind::HumanFrustration => "Human frustration",
        Kind::WrongAssumption => "Wrong assumption",
        Kind::WastedWork => "Wasted or redone work",
        Kind::Environment => "Environment or setup",
        Kind::MissingContext => "Missing context",
        Kind::Automatable => "Worth automating",
        Kind::KeyDecision => "Key decision",
        Kind::IgnoredInstruction => "Ignored instruction",
        Kind::Other => "Other",
    }
}

pub fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Unknown => "Unknown",
        Outcome::Succeeded => "Succeeded",
        Outcome::Partial => "Partial",
        Outcome::Failed => "Failed",
    }
}

fn category_label(c: ActionCategory) -> &'static str {
    match c {
        ActionCategory::Codebase => "Codebase",
        ActionCategory::Skill => "Skill",
        ActionCategory::Instructions => "Prompt or instructions",
        ActionCategory::Environment => "Environment",
        ActionCategory::Automation => "Automation",
    }
}

fn priority_label(p: Priority) -> &'static str {
    match p {
        Priority::High => "High",
        Priority::Medium => "Medium",
        Priority::Low => "Low",
    }
}

pub fn status_label(s: ActionStatus) -> &'static str {
    match s {
        ActionStatus::Done => "Done",
        ActionStatus::InProgress => "In progress",
        ActionStatus::Declined => "Declined",
        ActionStatus::Considering => "Thinking about it",
    }
}

fn impact_label(i: UserImpact) -> &'static str {
    match i {
        UserImpact::Helped => "Helped",
        UserImpact::DidNotHelp => "Didn’t help",
    }
}

fn verdict_label(v: Verdict) -> &'static str {
    match v {
        Verdict::Worked => "Worked",
        Verdict::DidNotWork => "Didn’t work",
        Verdict::TooEarly => "Too early to tell",
    }
}

/// Failures first: they are what the section exists to show.
fn verdict_order(v: Verdict) -> u8 {
    match v {
        Verdict::DidNotWork => 0,
        Verdict::Worked => 1,
        Verdict::TooEarly => 2,
    }
}

/// "claude-sonnet-5-5" → "Sonnet 5.5"; anything else as it is.
pub fn model_name(model: &str) -> String {
    let mut parts = model.strip_prefix("claude-").unwrap_or("").split('-');
    let (Some(name), Some(major), Some(minor)) = (parts.next(), parts.next(), parts.next()) else { return model.to_string() };
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let mut letters = name.chars();
    let Some(first) = letters.next().filter(|_| name.chars().all(|c| c.is_ascii_lowercase()) && digits(major) && digits(minor)) else {
        return model.to_string();
    };
    format!("{}{} {major}.{minor}", first.to_ascii_uppercase(), letters.as_str())
}

pub fn full_date(ms: i64) -> String {
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%a, %b %-d, %Y, %-I:%M %p").to_string(),
        None => "—".into(),
    }
}

pub fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the developer said about the report's actions, and what later reports found.
pub struct Responses {
    pub responses: Vec<ActionResponse>,
    pub verdicts: Vec<LatestVerdict>,
}

fn evidence_line(e: &Evidence, report: &Report, cited: &HashMap<(String, u32), &CitedAnnotation>) -> String {
    let key = match e {
        Evidence::Annotation { key, .. } | Evidence::Session { key } => key,
    };
    let session = report.sessions.iter().find(|s| &s.key == key);
    let title = session.map(|s| format!("“{}”", one_line(&s.title))).unwrap_or_else(|| "a session".into());
    match e {
        Evidence::Session { .. } => {
            let outcome = session.map(|s| outcome_label(s.outcome)).unwrap_or("Unknown");
            let note = session.and_then(|s| s.outcome_note.as_deref()).map(|n| format!(": {}", one_line(n))).unwrap_or_default();
            format!("{outcome} session {title}{note}")
        }
        Evidence::Annotation { id, version, .. } => match cited.get(&(id.clone(), *version)) {
            Some(c) => format!("{} in {title}: {}", kind_label(c.annotation.kind), one_line(&c.annotation.note)),
            None => format!("An annotation in {title}"),
        },
    }
}

/// Footnotes, numbered in the order evidence is first cited. One cited by
/// several takeaways or actions is one footnote referenced from each.
struct Footnotes<'a> {
    report: &'a Report,
    cited: HashMap<(String, u32), &'a CitedAnnotation>,
    labels: HashMap<String, usize>,
    text: Vec<String>,
}

impl Footnotes<'_> {
    fn refs(&mut self, list: &[Evidence]) -> String {
        let mut numbers: Vec<usize> = list
            .iter()
            .map(|e| {
                let id = match e {
                    Evidence::Session { key } => format!("s:{key}"),
                    Evidence::Annotation { id, version, .. } => format!("a:{id}@{version}"),
                };
                if let Some(n) = self.labels.get(&id) {
                    return *n;
                }
                self.text.push(evidence_line(e, self.report, &self.cited));
                self.labels.insert(id, self.text.len());
                self.text.len()
            })
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        // Spaced, or adjacent superscripts read as one number ("12" for 1 and 2). The "e"
        // keeps these apart from any footnote the model wrote.
        numbers.iter().map(|n| format!("[^e{n}]")).collect::<Vec<_>>().join(" ")
    }
}

pub fn report_markdown(report: &Report, cited: &[CitedAnnotation], responses: &Responses) -> String {
    let mut notes = Footnotes {
        report,
        cited: cited.iter().map(|c| ((c.annotation.id.clone(), c.annotation.version), c)).collect(),
        labels: HashMap::new(),
        text: Vec::new(),
    };
    let mut out: Vec<String> = vec![format!("# {}", report.title), String::new()];
    out.push(format!("{} · {} sessions · {}", full_date(report.created_at), report.sessions.len(), model_name(&report.model)));
    out.push(String::new());
    if let Some(focus) = &report.focus {
        out.push(format!("Focus: {}", one_line(focus)));
        out.push(String::new());
    }
    if !report.summary.is_empty() {
        out.push(report.summary.clone());
        out.push(String::new());
    }
    if !report.takeaways.is_empty() {
        out.push("## Key takeaways".into());
        out.push(String::new());
        for (i, t) in report.takeaways.iter().enumerate() {
            out.push(format!("{}. {}{}", i + 1, t.text, notes.refs(&t.evidence)));
        }
        out.push(String::new());
    }
    if !report.actions.is_empty() {
        out.push("## Recommended actions".into());
        out.push(String::new());
        for (i, a) in report.actions.iter().enumerate() {
            out.push(format!("### {}. {}", i + 1, a.title));
            out.push(String::new());
            out.push(format!("**{} priority** · {}", priority_label(a.priority), category_label(a.category)));
            out.push(String::new());
            // On a line of its own: the change is Markdown and may end in a list or code block.
            out.push(a.change.clone());
            out.push(String::new());
            if let Some(f) = &a.follows_up {
                out.push(format!("Follows up on: {}", one_line(&f.title)));
                out.push(String::new());
            }
            if let Some(check) = &a.success_check {
                out.push(format!("Success check: {}", one_line(check)));
                out.push(String::new());
            }
            if let Some(r) = responses.responses.iter().find(|r| r.action_index == i) {
                let status = r.status.map(status_label).unwrap_or("No status");
                let when = r.done_at.map(|at| format!(" on {}", full_date(at))).unwrap_or_default();
                let impact = r.user_impact.map(|u| format!(" ({})", impact_label(u))).unwrap_or_default();
                let note = r.note.as_deref().map(|n| format!(". Note: {}", one_line(n))).unwrap_or_default();
                out.push(format!("Status: {status}{when}{impact}{note}"));
                out.push(String::new());
            }
            if let Some(v) = responses.verdicts.iter().find(|v| v.action_index == i) {
                out.push(format!("Latest verdict: {}, per “{}”", verdict_label(v.verdict), one_line(&v.report_title)));
                out.push(String::new());
            }
            out.push(format!("Evidence: {}", notes.refs(&a.evidence)));
            out.push(String::new());
        }
    }
    if !report.impact.is_empty() {
        out.push("## Did earlier actions work?".into());
        out.push(String::new());
        let mut impact: Vec<_> = report.impact.iter().collect();
        impact.sort_by_key(|v| verdict_order(v.verdict));
        for v in impact {
            out.push(format!("### {}", v.action_title));
            out.push(String::new());
            out.push(format!("**{}.** {}", verdict_label(v.verdict), v.reasoning));
            out.push(String::new());
            if !v.evidence.is_empty() {
                out.push(format!("Evidence: {}", notes.refs(&v.evidence)));
                out.push(String::new());
            }
        }
    }
    out.push("## Sessions".into());
    out.push(String::new());
    for s in &report.sessions {
        out.push(format!("- {} ({})", one_line(&s.title), outcome_label(s.outcome).to_lowercase()));
    }
    if !notes.text.is_empty() {
        out.push(String::new());
        for (i, text) in notes.text.iter().enumerate() {
            out.push(format!("[^e{}]: {text}", i + 1));
        }
    }
    out.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_are_shortened_only_when_they_have_a_version() {
        assert_eq!(model_name("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(model_name("claude-haiku-5-5-20260101"), "Haiku 5.5");
        assert_eq!(model_name("sonnet"), "sonnet");
        assert_eq!(model_name("claude-sonnet"), "claude-sonnet");
        assert_eq!(model_name("gpt-5-2"), "gpt-5-2");
    }
}
