//! Version and changelog for the About window. The changelog is built in.

pub const NAME: &str = "AULA F87 Control";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEVELOPER: &str = "RadianceADM";
pub const WEBSITE: &str = "https://github.com/RadianceADM/aula-f87-control-rs";
pub const ISSUES: &str = "https://github.com/RadianceADM/aula-f87-control-rs/issues";
pub const CHANGELOG_URL: &str = "https://github.com/RadianceADM/aula-f87-control-rs/blob/main/CHANGELOG.md";
pub const SUMMARY: &str =
    "Backlight effects, per-key colors, music sync, sleep timer and battery level for AULA F87 / F87 Pro keyboards";
pub const COPYRIGHT: &str = "© 2026 RadianceADM";
pub const CHANGELOG: &str = include_str!("../CHANGELOG.md");

pub struct Release {
    pub version: String,
    pub date: String,
    /// (depth, Markdown text): depth 0 is a bullet, 1 a sub-bullet.
    pub items: Vec<(usize, String)>,
}

pub fn releases() -> Vec<Release> {
    let mut out: Vec<Release> = vec![];
    for line in CHANGELOG.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let rest = rest.trim().trim_start_matches('v');
            let (version, date) = match rest.split_once(['—', '–']) {
                Some((v, d)) => (v.trim(), d.trim()),
                None => (rest, ""),
            };
            out.push(Release { version: version.into(), date: date.into(), items: vec![] });
            continue;
        }
        let Some(release) = out.last_mut() else { continue };
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if let Some(text) = line.trim_start().strip_prefix("- ") {
            release.items.push(((indent / 2).min(1), text.trim().into()));
        } else if let Some((_, text)) = release.items.last_mut() {
            text.push(' ');
            text.push_str(line.trim()); // wrapped line of the previous bullet
        }
    }
    out
}

/// Markdown emphasis, `code` and [links](...) -> the given markup.
fn inline(text: &str, bold: fn(&str) -> String, italic: fn(&str) -> String, code: fn(&str) -> String) -> String {
    let mut out = String::new();
    let mut rest = text;
    // links: keep the text
    let mut linkless = String::new();
    while let Some(start) = rest.find('[') {
        let after = &rest[start..];
        match (after.find("]("), after.find(')')) {
            (Some(mid), Some(end)) if mid < end => {
                linkless.push_str(&rest[..start]);
                linkless.push_str(&after[1..mid]);
                rest = &after[end + 1..];
            }
            _ => {
                linkless.push_str(&rest[..start + 1]);
                rest = &rest[start + 1..];
            }
        }
    }
    linkless.push_str(rest);
    for (i, part) in linkless.split('`').enumerate() {
        if i % 2 == 1 {
            out.push_str(&code(part));
        } else {
            out.push_str(&emphasis(part, bold, italic));
        }
    }
    out
}

fn emphasis(text: &str, bold: fn(&str) -> String, italic: fn(&str) -> String) -> String {
    let mut out = String::new();
    for (i, part) in text.split("**").enumerate() {
        if i % 2 == 1 {
            out.push_str(&bold(part));
        } else {
            // *italic*: a star next to a non-space character on the inside
            let chars: Vec<char> = part.chars().collect();
            let mut j = 0;
            while j < chars.len() {
                if chars[j] == '*' && j + 1 < chars.len() && !chars[j + 1].is_whitespace() {
                    if let Some(k) = (j + 2..chars.len()).find(|&k| chars[k] == '*' && !chars[k - 1].is_whitespace()) {
                        out.push_str(&italic(&chars[j + 1..k].iter().collect::<String>()));
                        j = k + 1;
                        continue;
                    }
                }
                out.push(chars[j]);
                j += 1;
            }
        }
    }
    out
}

pub fn plain(text: &str) -> String {
    inline(text, |s| s.into(), |s| s.into(), |s| s.into())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// AppStream-style markup for AdwAboutDialog (p, ul, li, em, code). The dialog
/// already titles the notes with this version, so only older ones get a heading.
pub fn release_notes_markup() -> String {
    let mut out = String::new();
    for r in releases() {
        if r.version != VERSION {
            let title = if r.date.is_empty() { format!("Version {}", r.version) } else { format!("Version {} · {}", r.version, r.date) };
            out += &format!("<p><em>{}</em></p>", escape(&title));
        }
        if !r.items.is_empty() {
            out += "<ul>";
            for (_, text) in &r.items {
                out += &format!(
                    "<li>{}</li>",
                    inline(&escape(text), |s| format!("<em>{s}</em>"), |s| s.into(), |s| format!("<code>{s}</code>"))
                );
            }
            out += "</ul>";
        }
    }
    out
}
