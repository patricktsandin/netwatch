//! Writes a [`Report`] as plain text (for an email body) and as a printable,
//! self-contained HTML page that carries the same text for copying.

use std::fmt::Write as _;

use netwatch_probing::incident::Severity;
use netwatch_shared::{Role, WallTime};

use super::{Episode, PathStats, Place, format_duration, utc};

/// Everything a report says, decided by the daemon (ADR-0033).
#[derive(Debug, Clone)]
pub struct Report {
    /// The span's start.
    pub from: WallTime,
    /// The span's end.
    pub until: WallTime,
    /// When the report was made.
    pub generated: WallTime,
    /// The host's current identities, such as `("NIC", "eth0 (ifindex 5)")`.
    pub host: Vec<(String, String)>,
    /// Every episode in the span, with the path evidence measured during it.
    pub episodes: Vec<(Episode, Vec<PathStats>)>,
    /// Every role's measurements over the whole span.
    pub summary: Vec<PathStats>,
}

/// How roles are named, for people: target names come from the configuration.
pub type Label<'a> = &'a dyn Fn(&Role) -> String;

fn severity_text(severity: Severity) -> &'static str {
    match severity {
        Severity::Outage => "Outage",
        Severity::Degradation => "Degradation",
    }
}

fn place_text(place: Place) -> &'static str {
    match place {
        Place::BeyondGateway => "Beyond the gateway",
        Place::HostOrLan => "On this host or LAN",
    }
}

fn when(e: &Episode, until: WallTime) -> String {
    match e.ended {
        Some(end) => format!(
            "{} – {} ({})",
            utc(e.started),
            utc(end),
            format_duration(end.as_unix_micros() - e.started.as_unix_micros())
        ),
        None => format!(
            "{} – ongoing ({} so far)",
            utc(e.started),
            format_duration(until.as_unix_micros() - e.started.as_unix_micros())
        ),
    }
}

fn ms(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_owned(), |v| format!("{v:.1} ms"))
}

/// The episodes, beyond the gateway first, then oldest first.
fn ordered(report: &Report) -> Vec<&(Episode, Vec<PathStats>)> {
    let mut episodes: Vec<_> = report.episodes.iter().collect();
    episodes.sort_by_key(|(e, _)| (e.place, e.started));
    episodes
}

fn counts(report: &Report) -> (usize, usize) {
    let beyond = report
        .episodes
        .iter()
        .filter(|(e, _)| e.place == Place::BeyondGateway)
        .count();
    (beyond, report.episodes.len() - beyond)
}

fn stats_table(out: &mut String, stats: &[PathStats], label: Label<'_>) {
    let _ = writeln!(
        out,
        "  {:<40} {:<26} {:>7} {:>6} {:>10} {:>10}",
        "Role", "Identity", "Sent", "Loss", "Median", "p95"
    );
    for s in stats {
        let _ = writeln!(
            out,
            "  {:<40} {:<26} {:>7} {:>5.1}% {:>10} {:>10}",
            label(&s.role),
            s.identity.as_deref().unwrap_or("–"),
            s.sent,
            s.loss_percent(),
            ms(s.median_ms),
            ms(s.p95_ms)
        );
    }
}

/// The report as plain text, laid out in fixed-width columns for an email body.
pub fn text(report: &Report, label: Label<'_>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "netwatch incident report");
    let _ = writeln!(
        out,
        "Span:      {} – {} ({})",
        utc(report.from),
        utc(report.until),
        format_duration(report.until.as_unix_micros() - report.from.as_unix_micros())
    );
    let _ = writeln!(out, "Generated: {}", utc(report.generated));
    for (what, identity) in &report.host {
        let _ = writeln!(out, "{:<10} {identity}", format!("{what}:"));
    }
    let (beyond, local) = counts(report);
    let _ = writeln!(
        out,
        "\n{beyond} episode(s) beyond the gateway, {local} on this host or LAN."
    );
    for (n, (e, stats)) in ordered(report).into_iter().enumerate() {
        let _ = writeln!(
            out,
            "\n{}. {} · {}\n   {}",
            n + 1,
            severity_text(e.severity),
            place_text(e.place),
            when(e, report.until)
        );
        let _ = writeln!(out, "   Probable cause: {}", label(&e.cause));
        let affected: Vec<String> = e.affected.iter().map(label).collect();
        let _ = writeln!(out, "   Affected: {}", affected.join(", "));
        let _ = writeln!(out, "   Path during the episode:");
        stats_table(&mut out, stats, label);
    }
    let _ = writeln!(out, "\nWhole span, per role:");
    stats_table(&mut out, &report.summary, label);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn stats_html(out: &mut String, stats: &[PathStats], label: Label<'_>) {
    out.push_str(
        "<table><thead><tr><th>Role</th><th>Identity</th><th>Sent</th><th>Loss</th>\
         <th>Median</th><th>p95</th></tr></thead><tbody>",
    );
    for s in stats {
        let lossy = if s.lost > 0 { " class=\"lossy\"" } else { "" };
        let _ = write!(
            out,
            "<tr{lossy}><td>{}</td><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td><td>{}</td></tr>",
            escape(&label(&s.role)),
            escape(s.identity.as_deref().unwrap_or("–")),
            s.sent,
            s.loss_percent(),
            ms(s.median_ms),
            ms(s.p95_ms)
        );
    }
    out.push_str("</tbody></table>");
}

const STYLE: &str = "
:root { --bg: #fff; --text: #1d1f23; --muted: #6b7079; --border: #e4e4e1; --outage: #d9342b;
  --degradation: #b7791f; --lossy: #fdf0ef; color-scheme: light; }
@media (prefers-color-scheme: dark) { :root { --bg: #131417; --text: #e8e9ec; --muted: #9198a3;
  --border: #2c2f35; --outage: #ff6b61; --degradation: #e0a948; --lossy: #3a2321; color-scheme: dark; } }
body { margin: 0 auto; max-width: 1000px; padding: 24px 16px; background: var(--bg); color: var(--text);
  font: 14px/1.45 system-ui, sans-serif; }
h1 { font-size: 20px; margin: 0 0 4px; } h2 { font-size: 16px; margin: 28px 0 8px; }
.meta, .muted { color: var(--muted); } dl { display: grid; grid-template-columns: max-content 1fr; gap: 2px 12px; }
dt { color: var(--muted); } dd { margin: 0; }
.episode { border: 1px solid var(--border); border-left: 4px solid var(--degradation); border-radius: 6px;
  padding: 10px 14px; margin: 12px 0; break-inside: avoid; }
.episode.outage { border-left-color: var(--outage); }
.episode h3 { margin: 0 0 4px; font-size: 15px; }
table { border-collapse: collapse; width: 100%; margin-top: 8px; font-variant-numeric: tabular-nums; }
th, td { text-align: left; padding: 3px 8px; border-bottom: 1px solid var(--border); }
th:nth-child(n+3), td:nth-child(n+3) { text-align: right; }
tr.lossy td { background: var(--lossy); }
.scroll { overflow-x: auto; }
pre { white-space: pre; overflow-x: auto; border: 1px solid var(--border); border-radius: 6px; padding: 10px; }
button { font: inherit; padding: 4px 12px; border: 1px solid var(--border); border-radius: 6px;
  background: var(--bg); color: var(--text); cursor: pointer; }
@media print { button, details { display: none; } body { max-width: none; } }
";

/// The report as a self-contained HTML page: printable (or saved as PDF from the
/// browser), with the plain text alongside for copying into an email.
pub fn html(report: &Report, label: Label<'_>) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Incident report</title><style>{STYLE}</style></head><body>"
    );
    let _ = write!(
        out,
        "<h1>Network incident report</h1><p class=\"meta\">{} – {} ({}) · generated {}</p><dl>",
        utc(report.from),
        utc(report.until),
        format_duration(report.until.as_unix_micros() - report.from.as_unix_micros()),
        utc(report.generated)
    );
    for (what, identity) in &report.host {
        let _ = write!(
            out,
            "<dt>{}</dt><dd>{}</dd>",
            escape(what),
            escape(identity)
        );
    }
    let (beyond, local) = counts(report);
    let _ = write!(
        out,
        "</dl><p><strong>{beyond}</strong> episode(s) beyond the gateway, \
         <strong>{local}</strong> on this host or LAN.</p>\
         <p><button id=\"copy\">Copy as text</button> <button onclick=\"print()\">Print or save as PDF</button></p>"
    );
    if report.episodes.is_empty() {
        out.push_str("<p class=\"muted\">No incidents in this span.</p>");
    }
    for (n, (e, stats)) in ordered(report).into_iter().enumerate() {
        let kind = if e.severity == Severity::Outage {
            "outage"
        } else {
            "degradation"
        };
        let affected: Vec<String> = e.affected.iter().map(|r| escape(&label(r))).collect();
        let _ = write!(
            out,
            "<section class=\"episode {kind}\"><h3>{}. {} · {}</h3><div>{}</div>\
             <div>Probable cause: <strong>{}</strong></div><div class=\"muted\">Affected: {}</div>\
             <div class=\"scroll\">",
            n + 1,
            severity_text(e.severity),
            place_text(e.place),
            escape(&when(e, report.until)),
            escape(&label(&e.cause)),
            affected.join(", ")
        );
        stats_html(&mut out, stats, label);
        out.push_str("</div></section>");
    }
    out.push_str("<h2>Whole span, per role</h2><div class=\"scroll\">");
    stats_html(&mut out, &report.summary, label);
    let _ = write!(
        out,
        "</div><details><summary>Plain text</summary><pre id=\"text\">{}</pre></details>\
         <script>document.getElementById('copy').onclick = () => \
         navigator.clipboard.writeText(document.getElementById('text').textContent);</script>\
         </body></html>",
        escape(&text(report, label))
    );
    out
}
