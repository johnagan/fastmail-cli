use pulldown_cmark::{html, Options, Parser};
use std::collections::BTreeMap;

/// How content is supplied: markdown (default), HTML, or plain text.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Format {
    Markdown,
    Html,
    Text,
}

/// Content in the forms a message needs. `text` is always present.
/// `html` is None for plain-text input, where no HTML part is needed.
pub struct Body {
    pub text: String,
    pub html: Option<String>,
}

pub fn body(format: Format, input: &str) -> Body {
    match format {
        Format::Markdown => Body { text: input.to_string(), html: Some(document(&to_html(input))) },
        Format::Html => Body {
            text: to_markdown(input).markdown,
            html: Some(if input.to_ascii_lowercase().contains("<html") { input.to_string() } else { document(input) }),
        },
        Format::Text => Body { text: input.to_string(), html: None },
    }
}

/// Wraps an HTML fragment in a full document.
pub fn document(fragment: &str) -> String {
    format!("<html><head><meta charset=\"utf-8\"></head><body>{fragment}</body></html>")
}

/// Plain text as HTML: escaped, one block per line. Used for notes written as plain text.
#[cfg(feature = "notes")]
pub fn text_to_html(text: &str) -> String {
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            let e = l.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
            format!("<div>{}</div>", if e.is_empty() { "<br>".to_string() } else { e })
        })
        .collect();
    document(&lines.join(""))
}

pub struct Converted {
    pub markdown: String,
    /// Formatting in the HTML that markdown can't express, counted by kind.
    #[cfg_attr(not(feature = "notes"), allow(dead_code))]
    pub warnings: Vec<String>,
}

/// Markdown to HTML. CommonMark plus tables, strikethrough, and task lists.
pub fn to_html(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));
    out
}

/// Tags with no visible effect that are safe to skip silently.
const IGNORED: &[&str] = &["html", "head", "body", "meta", "title", "span", "tbody", "thead", "o:p"];

/// Tags that map to markdown structure.
const STRUCTURAL: &[&str] = &[
    "p", "div", "br", "h1", "h2", "h3", "h4", "h5", "h6", "b", "strong", "i", "em", "ul", "ol", "li", "a", "tr",
];

/// HTML to markdown for the common subset. Anything else is dropped and reported in `warnings`.
pub fn to_markdown(html: &str) -> Converted {
    let cleaned = strip_blocks(html);
    let mut conv = Converter::default();
    let mut rest = cleaned.as_str();
    while let Some(lt) = rest.find('<') {
        conv.text(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>').map(|p| lt + p) else { break };
        conv.tag(&rest[lt + 1..gt]);
        rest = &rest[gt + 1..];
    }
    conv.text(rest);
    conv.finish()
}

fn strip_blocks(html: &str) -> String {
    let mut s = html.replace("\r\n", "\n");
    for (open, close) in [("<head", "</head>"), ("<style", "</style>"), ("<script", "</script>")] {
        while let Some(start) = s.to_ascii_lowercase().find(open) {
            let end = s[start..].to_ascii_lowercase().find(close).map(|p| start + p + close.len()).unwrap_or(s.len());
            s.replace_range(start..end, "");
        }
    }
    while let Some(start) = s.find("<!--") {
        let end = s[start..].find("-->").map(|p| start + p + 3).unwrap_or(s.len());
        s.replace_range(start..end, "");
    }
    s
}

#[derive(Default)]
struct Converter {
    out: String,
    warnings: BTreeMap<String, usize>,
    lists: Vec<(bool, u32)>,
    links: Vec<(usize, String)>,
}

impl Converter {
    fn newline(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    /// Starts a new block: a blank line before it, unless nothing has been written yet.
    fn block(&mut self) {
        if self.out.is_empty() {
            return;
        }
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        if !self.out.ends_with("\n\n") {
            if !self.out.ends_with('\n') {
                self.out.push('\n');
            }
            self.out.push('\n');
        }
    }

    fn warn(&mut self, what: String) {
        *self.warnings.entry(what).or_insert(0) += 1;
    }

    fn text(&mut self, raw: &str) {
        let decoded = decode_entities(raw);
        let words: Vec<&str> = decoded.split_whitespace().collect();
        let needs_space = !self.out.is_empty() && !self.out.ends_with(['\n', ' ']);
        if words.is_empty() {
            if !decoded.is_empty() && needs_space {
                self.out.push(' ');
            }
            return;
        }
        if decoded.starts_with(char::is_whitespace) && needs_space {
            self.out.push(' ');
        }
        self.out.push_str(&escape(&words.join(" ")));
        if decoded.ends_with(char::is_whitespace) {
            self.out.push(' ');
        }
    }

    fn tag(&mut self, raw: &str) {
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_end_matches('/').trim();
        let name = body.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_ascii_lowercase();
        if name.starts_with('!') || name.starts_with('?') {
            return;
        }
        let attrs = &body[name.len().min(body.len())..];
        self.style_warning(&name, attrs);

        if !STRUCTURAL.contains(&name.as_str()) {
            if name == "img" {
                self.warn("image (not kept)".into());
            } else if !IGNORED.contains(&name.as_str()) && !closing {
                self.warn(format!("unsupported <{name}>"));
            }
            return;
        }

        match (name.as_str(), closing) {
            ("br", _) => self.out.push('\n'),
            ("p" | "div" | "tr", _) => self.block(),
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", false) => {
                self.block();
                let level = name[1..].parse::<usize>().unwrap_or(1);
                self.out.push_str(&"#".repeat(level));
                self.out.push(' ');
            }
            ("h1" | "h2" | "h3" | "h4" | "h5" | "h6", true) => self.block(),
            ("b" | "strong", _) => self.out.push_str("**"),
            ("i" | "em", _) => self.out.push('*'),
            ("ul", false) => {
                self.block();
                self.lists.push((false, 0));
            }
            ("ol", false) => {
                self.block();
                self.lists.push((true, 0));
            }
            ("ul" | "ol", true) => {
                self.lists.pop();
                self.block();
            }
            ("li", false) => {
                self.newline();
                let depth = self.lists.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let marker = match self.lists.last_mut() {
                    Some((true, n)) => {
                        *n += 1;
                        format!("{n}. ")
                    }
                    _ => "- ".to_string(),
                };
                self.out.push_str(&indent);
                self.out.push_str(&marker);
            }
            ("li", true) => self.newline(),
            ("a", false) => {
                let href = attr(attrs, "href").unwrap_or_default();
                self.links.push((self.out.len(), href));
            }
            ("a", true) => {
                if let Some((start, href)) = self.links.pop() {
                    let text = self.out.split_off(start);
                    let text = text.trim();
                    if href.is_empty() {
                        self.out.push_str(text);
                    } else {
                        self.out.push_str(&format!("[{text}]({href})"));
                    }
                }
            }
            _ => {}
        }
    }

    fn style_warning(&mut self, name: &str, attrs: &str) {
        if let Some(style) = attr(attrs, "style") {
            let props: Vec<&str> = style
                .split(';')
                .filter_map(|p| p.split(':').next())
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .collect();
            if !props.is_empty() {
                self.warn(format!("style on <{name}> ({})", props.join(", ")));
            }
        }
    }

    fn finish(self) -> Converted {
        let mut md = self.out.replace("\r\n", "\n");
        md = md
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        while md.contains("\n\n\n") {
            md = md.replace("\n\n\n", "\n\n");
        }
        Converted {
            markdown: md.trim().to_string() + "\n",
            warnings: self.warnings.into_iter().map(|(k, n)| if n > 1 { format!("{k} x{n}") } else { k }).collect(),
        }
    }
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let start = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &attrs[start..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = rest[1..].find(quote)? + 1;
        Some(rest[1..end].to_string())
    } else {
        Some(rest.split_whitespace().next().unwrap_or("").to_string())
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '_' | '[' | ']' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&p| p <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16).ok().and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_structure() {
        let html = "<html><body><h2>Kitchen</h2><ul><li>Knife</li><li>Board <b>large</b></li></ul><p><a href=\"https://x.io\">site</a></p></body></html>";
        let c = to_markdown(html);
        assert_eq!(c.markdown, "## Kitchen\n\n- Knife\n- Board **large**\n\n[site](https://x.io)\n");
        assert!(c.warnings.is_empty());
    }

    #[test]
    fn reports_styling_it_cannot_keep() {
        let c = to_markdown("<div style=\"color: red; font-size: 20px\">Hi</div><img src=\"x\">");
        assert!(c.warnings.iter().any(|w| w.contains("color, font-size")));
        assert!(c.warnings.iter().any(|w| w.contains("image")));
    }

    #[test]
    fn markdown_round_trips_through_html() {
        let md = "# Title\n\n- one\n- two\n\nSome **bold** text.\n";
        let back = to_markdown(&to_html(md));
        assert_eq!(back.markdown, md);
    }

    #[test]
    fn decodes_entities() {
        assert_eq!(decode_entities("a &amp; b &#39;c&#39; &#x2192;"), "a & b 'c' \u{2192}");
    }

    #[test]
    fn plain_text_has_no_html_part() {
        let b = body(Format::Text, "hello *not bold*");
        assert_eq!(b.text, "hello *not bold*");
        assert!(b.html.is_none());
    }

    #[test]
    fn markdown_and_html_both_produce_text_and_html() {
        let md = body(Format::Markdown, "# Hi");
        assert_eq!(md.text, "# Hi");
        assert!(md.html.unwrap().contains("<h1>Hi</h1>"));
        let html = body(Format::Html, "<h2>Title</h2><p>Body</p>");
        assert!(html.text.contains("## Title"));
        assert!(html.html.unwrap().contains("<h2>Title</h2>"));
    }
}
