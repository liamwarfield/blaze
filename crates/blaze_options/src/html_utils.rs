//! Port of `HtmlUtils` (utilities related to HTML generation), with `lib.util.Markdown` and
//! Guava's `HtmlEscapers.htmlEscaper()`, which it uses.

use crate::data::OptionsData;
use crate::definition::OptionDefinitionRef;
use crate::option_filter_descriptions::OptionFilterDescriptions;
use crate::options_usage::OptionsUsage;
use crate::parser::OptionsParser;
use crate::tags::OptionEffectTag;

/// Guava's `Escaper`, as `HtmlUtils` uses it.
pub type Escaper = fn(&str) -> String;

/// Guava `HtmlEscapers.htmlEscaper()`: escapes `"`, `'`, `&`, `<` and `>`.
pub fn html_escaper(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("&quot;"),
            // Note: "&apos;" is not defined in HTML 4.01.
            '\'' => out.push_str("&#39;"),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Helpers for markdown.
// Java: com/google/devtools/build/lib/util/Markdown.java:21 Markdown
pub struct Markdown;

impl Markdown {
    /// Parses a string as markdown and renders it as HTML.
    ///
    /// PORT: Java uses commonmark-java 0.25 (`Parser.builder().build()`, `HtmlRenderer` with
    /// `escapeHtml(true)`): plain CommonMark without extensions. Blaze parses with pulldown-cmark
    /// (no extensions; both follow the CommonMark spec) and renders the events with a port of
    /// commonmark-java's `CoreHtmlNodeRenderer` and `HtmlWriter` ([`CommonmarkHtmlWriter`]).
    // Java: com/google/devtools/build/lib/util/Markdown.java:26 Markdown.renderToHtml
    pub fn render_to_html(md: &str) -> String {
        let parser = pulldown_cmark::Parser::new_ext(md, pulldown_cmark::Options::empty());
        let mut writer = CommonmarkHtmlWriter::default();
        writer.render(parser);
        writer.buffer.trim_matches(|c: char| c <= ' ').to_string()
    }
}

/// commonmark-java's `HtmlWriter` and the parts of `CoreHtmlNodeRenderer` (0.25.0) that the
/// default `HtmlRenderer` with `escapeHtml(true)` uses: no URL sanitizing, no percent-encoding
/// of URLs, soft breaks as `\n`.
#[derive(Default)]
struct CommonmarkHtmlWriter {
    buffer: String,
    /// Image alt text being collected (`AltTextVisitor`), with the image's nesting depth.
    alt_text: Option<(String, usize)>,
    image_attrs: Vec<(String, Option<String>)>,
}

impl CommonmarkHtmlWriter {
    /// `Escaping.escapeHtml`.
    fn escape_html(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                c => out.push(c),
            }
        }
        out
    }

    /// `HtmlWriter.raw`.
    fn raw(&mut self, s: &str) {
        self.buffer.push_str(s);
    }

    /// `HtmlWriter.text`.
    fn text(&mut self, s: &str) {
        let escaped = Self::escape_html(s);
        self.buffer.push_str(&escaped);
    }

    /// `HtmlWriter.tag(name, attrs, voidElement)`.
    fn tag(&mut self, name: &str, attrs: &[(&str, String)], void_element: bool) {
        self.buffer.push('<');
        self.buffer.push_str(name);
        for (key, value) in attrs {
            self.buffer.push(' ');
            self.buffer.push_str(&Self::escape_html(key));
            self.buffer.push_str("=\"");
            self.buffer.push_str(&Self::escape_html(value));
            self.buffer.push('"');
        }
        if void_element {
            self.buffer.push_str(" /");
        }
        self.buffer.push('>');
    }

    /// `HtmlWriter.line`.
    fn line(&mut self) {
        if !self.buffer.is_empty() && !self.buffer.ends_with('\n') {
            self.buffer.push('\n');
        }
    }

    fn render<'a>(&mut self, events: impl Iterator<Item = pulldown_cmark::Event<'a>>) {
        use pulldown_cmark::{CodeBlockKind, Event, Tag, TagEnd};
        for event in events {
            // AltTextVisitor: inside an image only text and line breaks count.
            if let Some((alt, depth)) = &mut self.alt_text {
                match &event {
                    Event::Start(Tag::Image { .. }) => *depth += 1,
                    Event::End(TagEnd::Image) if *depth > 0 => *depth -= 1,
                    Event::End(TagEnd::Image) => {
                        let alt = std::mem::take(alt);
                        self.alt_text = None;
                        let (src, title) = self.image_attrs.pop().expect("image attributes");
                        let mut attrs = vec![("src", src), ("alt", alt)];
                        if let Some(title) = title {
                            attrs.push(("title", title));
                        }
                        self.tag("img", &attrs, true);
                    }
                    Event::Text(t) => alt.push_str(t),
                    Event::SoftBreak | Event::HardBreak => alt.push('\n'),
                    _ => {}
                }
                continue;
            }
            match event {
                Event::Start(tag) => match tag {
                    Tag::Paragraph => {
                        self.line();
                        self.tag("p", &[], false);
                    }
                    Tag::Heading { level, .. } => {
                        self.line();
                        self.tag(&format!("h{}", level as usize), &[], false);
                    }
                    Tag::BlockQuote(_) => {
                        self.line();
                        self.tag("blockquote", &[], false);
                        self.line();
                    }
                    Tag::CodeBlock(kind) => {
                        self.line();
                        self.tag("pre", &[], false);
                        let mut attrs = Vec::new();
                        if let CodeBlockKind::Fenced(info) = kind
                            && !info.is_empty()
                        {
                            let language = match info.find(' ') {
                                Some(i) => &info[..i],
                                None => &info[..],
                            };
                            attrs.push(("class", format!("language-{}", language)));
                        }
                        self.tag("code", &attrs, false);
                    }
                    Tag::HtmlBlock => {
                        // escapeHtml: the literal is shown as text in a paragraph.
                        self.line();
                        self.tag("p", &[], false);
                    }
                    Tag::List(start) => {
                        self.line();
                        match start {
                            None => self.tag("ul", &[], false),
                            Some(n) if n != 1 => self.tag("ol", &[("start", n.to_string())], false),
                            Some(_) => self.tag("ol", &[], false),
                        }
                        self.line();
                    }
                    Tag::Item => self.tag("li", &[], false),
                    Tag::Emphasis => self.tag("em", &[], false),
                    Tag::Strong => self.tag("strong", &[], false),
                    Tag::Link {
                        dest_url, title, ..
                    } => {
                        let mut attrs = vec![("href", dest_url.to_string())];
                        if !title.is_empty() {
                            attrs.push(("title", title.to_string()));
                        }
                        self.tag("a", &attrs, false);
                    }
                    Tag::Image {
                        dest_url, title, ..
                    } => {
                        self.image_attrs.push((
                            dest_url.to_string(),
                            (!title.is_empty()).then(|| title.to_string()),
                        ));
                        self.alt_text = Some((String::new(), 0));
                    }
                    other => blaze_util::unported!(
                        "Markdown.renderToHtml",
                        "CommonMark node {:?} (an extension Bazel does not enable)",
                        other
                    ),
                },
                Event::End(tag) => match tag {
                    TagEnd::Paragraph => {
                        self.tag("/p", &[], false);
                        self.line();
                    }
                    TagEnd::Heading(level) => {
                        self.tag(&format!("/h{}", level as usize), &[], false);
                        self.line();
                    }
                    TagEnd::BlockQuote(_) => {
                        self.line();
                        self.tag("/blockquote", &[], false);
                        self.line();
                    }
                    TagEnd::CodeBlock => {
                        self.tag("/code", &[], false);
                        self.tag("/pre", &[], false);
                        self.line();
                    }
                    TagEnd::HtmlBlock => {
                        // HtmlBlock.getLiteral() joins the lines without a final newline.
                        if self.buffer.ends_with('\n') {
                            self.buffer.pop();
                        }
                        self.tag("/p", &[], false);
                        self.line();
                    }
                    TagEnd::List(ordered) => {
                        self.line();
                        self.tag(if ordered { "/ol" } else { "/ul" }, &[], false);
                        self.line();
                    }
                    TagEnd::Item => {
                        self.tag("/li", &[], false);
                        self.line();
                    }
                    TagEnd::Emphasis => self.tag("/em", &[], false),
                    TagEnd::Strong => self.tag("/strong", &[], false),
                    TagEnd::Link => self.tag("/a", &[], false),
                    other => blaze_util::unported!(
                        "Markdown.renderToHtml",
                        "CommonMark node end {:?}",
                        other
                    ),
                },
                Event::Text(t) => self.text(&t),
                Event::Code(t) => {
                    self.tag("code", &[], false);
                    self.text(&t);
                    self.tag("/code", &[], false);
                }
                // escapeHtml(true): raw HTML is written as text.
                Event::Html(t) | Event::InlineHtml(t) => self.text(&t),
                Event::SoftBreak => self.raw("\n"),
                Event::HardBreak => {
                    self.tag("br", &[], true);
                    self.line();
                }
                Event::Rule => {
                    self.line();
                    self.tag("hr", &[], true);
                    self.line();
                }
                other => {
                    blaze_util::unported!("Markdown.renderToHtml", "CommonMark event {:?}", other)
                }
            }
        }
    }
}

/// Utilities related to HTML generation.
// Java: com/google/devtools/common/options/HtmlUtils.java:31 HtmlUtils
pub struct HtmlUtils;

impl HtmlUtils {
    /// Returns a description of all the options the given parser can digest. In addition to
    /// `Option` annotations, this method also interprets `OptionsUsage` annotations which give an
    /// intuitive short description for the options.
    // Java: com/google/devtools/common/options/HtmlUtils.java:39 HtmlUtils.describeOptionsHtml
    pub fn describe_options_html(
        parser: &OptionsParser,
        escaper: Escaper,
        options_to_ignore: &[String],
        command_name: Option<&str>,
    ) -> String {
        let mut desc = String::new();
        let option_category_descriptions =
            OptionFilterDescriptions::get_option_categories_enum_description();

        for (category, categorized_options_list) in parser.get_options_sorted_by_category() {
            let categorized_options_list: Vec<OptionDefinitionRef> = categorized_options_list
                .into_iter()
                .filter(|option_def| {
                    !option_def
                        .get_option_effect_tags()
                        .contains(&OptionEffectTag::NO_OP)
                })
                .filter(|option_def| {
                    !options_to_ignore
                        .iter()
                        .any(|n| n == option_def.get_option_name())
                })
                .collect();
            if categorized_options_list.is_empty() {
                continue;
            }
            let category_description = &option_category_descriptions[&category];

            desc.push_str("<dl>");
            desc.push_str(&escaper(category_description));
            desc.push_str(":\n");
            for option_def in categorized_options_list {
                let class = parser
                    .options_data()
                    .get_options_classes()
                    .iter()
                    .copied()
                    .find(|c| c.type_id() == option_def.declaring_class_id())
                    .expect("the declaring class of an option of the parser");
                let data = OptionsData::get(&[class], false).unwrap_or_else(|e| e.throw());
                Self::get_usage_html(option_def, &mut desc, escaper, &data, true, command_name);
            }
            desc.push_str("</dl>\n");
        }
        desc
    }

    /// Append the usage message for a single option-field message to 'usage'.
    // Java: com/google/devtools/common/options/HtmlUtils.java:75 HtmlUtils.getUsageHtml
    pub fn get_usage_html(
        option_definition: OptionDefinitionRef,
        usage: &mut String,
        escaper: Escaper,
        options_data: &OptionsData,
        include_tags: bool,
        command_name: Option<&str>,
    ) {
        let plain_flag_name = option_definition.get_option_name();
        let flag_name = OptionsUsage::get_flag_name(option_definition);
        let value_description = option_definition.get_value_type_help_text();
        let type_description = OptionsUsage::get_type_description(option_definition);

        let mut anchor_id = String::new();
        if let Some(command_name) = command_name {
            anchor_id.push_str(command_name);
            anchor_id.push('-');
        }
        anchor_id.push_str("flag--");
        anchor_id.push_str(plain_flag_name);

        usage.push_str("<dt id=\"");
        // Add the id of the flag to point anchor hrefs to it
        usage.push_str(&anchor_id);
        usage.push_str("\">");
        // Add the href to the id hash
        usage.push_str("<code");
        // Historically, we used `flag--${plainFlagName}` as the anchor id, but this is not unique
        // across commands. We now use the per-command `anchorId` defined above, but we moved the
        // old `flag--${plainFlagName}` to be an id on the code block for backwards compatibility
        // with old links.
        if command_name.is_some() {
            usage.push_str(" id=\"");
            usage.push_str(plain_flag_name);
            usage.push('"');
        }
        usage.push('>');
        usage.push_str("<a href=\"#");
        usage.push_str(&anchor_id);
        usage.push_str("\">");
        usage.push_str("--");
        usage.push_str(&flag_name);
        usage.push_str("</a>");

        if !option_definition.requires_value() {
            // Nothing for boolean, tristate, boolean_or_enum, or void options.
        } else if !value_description.is_empty() {
            usage.push('=');
            usage.push_str(&escaper(value_description));
        } else if !type_description.is_empty() {
            // Generic fallback, which isn't very good.
            usage.push_str("=&lt;");
            usage.push_str(&escaper(&type_description));
            usage.push_str("&gt;");
        }
        usage.push_str("</code>");
        if option_definition.get_abbreviation() != '\0' {
            usage.push_str(" [<code>-");
            usage.push(option_definition.get_abbreviation());
            usage.push_str("</code>]");
        }
        if option_definition.allows_multiple() {
            // Allow-multiple options can't have a default value.
            usage.push_str(" multiple uses are accumulated");
        } else {
            // Don't call the annotation directly (we must allow overrides to certain defaults).
            let default_value_string = option_definition.get_unparsed_default_value();
            if option_definition.is_void_field() {
                // Void options don't have a default.
            } else if option_definition.is_special_null_default() {
                usage.push_str(" default: see description");
            } else {
                usage.push_str(" default: \"");
                usage.push_str(&escaper(default_value_string));
                usage.push('"');
            }
        }
        usage.push_str("</dt>\n");
        usage.push_str("<dd>\n");
        if !option_definition.get_help_text().is_empty() {
            usage.push_str(&Markdown::render_to_html(option_definition.get_help_text()));
            usage.push('\n');
        }

        let expansion = options_data.get_evaluated_expansion(option_definition);
        if !expansion.is_empty() {
            // If this is an expansion option, list the expansion if known, or at least specify
            // that we don't know.
            usage.push_str("<p>Expands to:\n");
            for exp in &expansion {
                // TODO(jingwen): We link to the expanded flags here, but unfortunately we don't
                // currently guarantee that all flags are only printed once. A flag in an
                // OptionBase that is included by 2 different commands, but not inherited through
                // a parent command, will be printed multiple times. Clicking on the flag will
                // bring the user to its first definition.
                let escaped = escaper(exp);
                usage.push_str("<br/>&nbsp;&nbsp;");
                usage.push_str("<code><a href=\"#flag");
                // Link to the '#flag--flag_name' hash.
                // Some expansions are in the form of '--flag_name=value', so we drop everything
                // from '=' onwards.
                usage.push_str(escaped.split('=').next().unwrap_or(""));
                usage.push_str("\">");
                usage.push_str(&escaped);
                usage.push_str("</a></code>\n");
            }
            usage.push_str("</p>");
        }

        // Add effect tags, if not UNKNOWN, and metadata tags, if not empty.
        if include_tags {
            let tag_list: Vec<String> = option_definition
                .get_option_effect_tags()
                .iter()
                .filter(|t| OptionsUsage::should_effect_tag_be_listed(**t))
                .map(|tag| {
                    format!(
                        "<a href=\"#effect_tag_{}\"><code>{}</code></a>",
                        tag.name(),
                        tag.name().to_ascii_lowercase()
                    )
                })
                .chain(
                    option_definition
                        .get_option_metadata_tags()
                        .iter()
                        .filter(|t| OptionsUsage::should_metadata_tag_be_listed(**t))
                        .map(|tag| {
                            format!(
                                "<a href=\"#metadata_tag_{}\"><code>{}</code></a>",
                                tag.name(),
                                tag.name().to_ascii_lowercase()
                            )
                        }),
                )
                .collect();
            let tag_list = tag_list.join(", ");
            if !tag_list.is_empty() {
                usage.push_str("<p>Tags:\n");
                usage.push_str(&tag_list);
                usage.push_str("\n</p>");
            }
        }

        usage.push_str("</dd>\n");
    }
}
