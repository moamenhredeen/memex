use crate::markdown::{StyleKind, StyleSpan};
use std::cell::RefCell;
use std::sync::OnceLock;
use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "character",
    "comment",
    "comment.documentation",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "escape",
    "function",
    "function.builtin",
    "function.call",
    "function.macro",
    "function.method",
    "function.method.call",
    "keyword",
    "keyword.conditional",
    "keyword.coroutine",
    "keyword.debug",
    "keyword.directive",
    "keyword.exception",
    "keyword.function",
    "keyword.import",
    "keyword.modifier",
    "keyword.operator",
    "keyword.repeat",
    "keyword.return",
    "keyword.type",
    "label",
    "module",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.documentation",
    "string.escape",
    "string.regexp",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.member",
    "variable.parameter",
];

// tree-sitter-kotlin-ng currently packages its parser without a highlight query.
const KOTLIN_HIGHLIGHTS_QUERY: &str = r#"
[(line_comment) (block_comment)] @comment
[(string_literal) (multiline_string_literal)] @string
(character_literal) @character
(escape_sequence) @string.escape
[(number_literal) (float_literal)] @number
(identifier) @variable
[
  "as" "catch" "class" "do" "else" "finally" "for" "fun" "if" "import"
  "in" "is" "package" "return" "this" "throw" "try" "val" "var" "when"
  "while"
] @keyword
"#;

pub fn highlight(language: &str, source: &str) -> Option<Vec<StyleSpan>> {
    let config = configuration(language)?;

    thread_local! {
        static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
    }

    HIGHLIGHTER.with_borrow_mut(|highlighter| {
        let events = highlighter
            .highlight(config, source.as_bytes(), None, |_| None)
            .ok()?;
        let mut spans = Vec::new();
        let mut active = Vec::new();

        for event in events {
            match event.ok()? {
                HighlightEvent::HighlightStart(highlight) => active.push(highlight.0),
                HighlightEvent::HighlightEnd => {
                    active.pop();
                }
                HighlightEvent::Source { start, end } if end > start => {
                    let kind = active
                        .last()
                        .map(|index| style_for_capture(HIGHLIGHT_NAMES[*index]))
                        .unwrap_or(StyleKind::Code);
                    push_span(&mut spans, start, end, kind);
                }
                HighlightEvent::Source { .. } => {}
            }
        }

        Some(spans)
    })
}

fn configuration(language: &str) -> Option<&'static HighlightConfiguration> {
    static POWERSHELL: OnceLock<HighlightConfiguration> = OnceLock::new();
    static BASH: OnceLock<HighlightConfiguration> = OnceLock::new();
    static FISH: OnceLock<HighlightConfiguration> = OnceLock::new();
    static TOML: OnceLock<HighlightConfiguration> = OnceLock::new();
    static JSON: OnceLock<HighlightConfiguration> = OnceLock::new();
    static YAML: OnceLock<HighlightConfiguration> = OnceLock::new();
    static JAVA: OnceLock<HighlightConfiguration> = OnceLock::new();
    static CSHARP: OnceLock<HighlightConfiguration> = OnceLock::new();
    static HTML: OnceLock<HighlightConfiguration> = OnceLock::new();
    static CSS: OnceLock<HighlightConfiguration> = OnceLock::new();
    static JAVASCRIPT: OnceLock<HighlightConfiguration> = OnceLock::new();
    static TYPESCRIPT: OnceLock<HighlightConfiguration> = OnceLock::new();
    static TSX: OnceLock<HighlightConfiguration> = OnceLock::new();
    static PYTHON: OnceLock<HighlightConfiguration> = OnceLock::new();
    static C: OnceLock<HighlightConfiguration> = OnceLock::new();
    static RUST: OnceLock<HighlightConfiguration> = OnceLock::new();
    static ZIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    static MAKE: OnceLock<HighlightConfiguration> = OnceLock::new();
    static XML: OnceLock<HighlightConfiguration> = OnceLock::new();
    static KOTLIN: OnceLock<HighlightConfiguration> = OnceLock::new();
    static DART: OnceLock<HighlightConfiguration> = OnceLock::new();

    let language = language.trim().to_ascii_lowercase();
    match language.as_str() {
        "powershell" | "pwsh" | "ps1" => Some(POWERSHELL.get_or_init(|| {
            build_config(
                tree_sitter_powershell::LANGUAGE.into(),
                "powershell",
                tree_sitter_powershell::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "bash" | "sh" | "shell" => Some(BASH.get_or_init(|| {
            build_config(
                tree_sitter_bash::LANGUAGE.into(),
                "bash",
                tree_sitter_bash::HIGHLIGHT_QUERY,
                "",
                "",
            )
        })),
        "fish" => Some(FISH.get_or_init(|| {
            build_config(
                tree_sitter_fish::language(),
                "fish",
                tree_sitter_fish::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "toml" => Some(TOML.get_or_init(|| {
            build_config(
                tree_sitter_toml_ng::LANGUAGE.into(),
                "toml",
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "json" | "jsonc" => Some(JSON.get_or_init(|| {
            build_config(
                tree_sitter_json::LANGUAGE.into(),
                "json",
                tree_sitter_json::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "yaml" | "yml" => Some(YAML.get_or_init(|| {
            build_config(
                tree_sitter_yaml::LANGUAGE.into(),
                "yaml",
                tree_sitter_yaml::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "java" => Some(JAVA.get_or_init(|| {
            build_config(
                tree_sitter_java::LANGUAGE.into(),
                "java",
                tree_sitter_java::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "csharp" | "cs" | "c#" => Some(CSHARP.get_or_init(|| {
            build_config(
                tree_sitter_c_sharp::LANGUAGE.into(),
                "csharp",
                tree_sitter_c_sharp::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "html" | "htm" => Some(HTML.get_or_init(|| {
            build_config(
                tree_sitter_html::LANGUAGE.into(),
                "html",
                tree_sitter_html::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "css" => Some(CSS.get_or_init(|| {
            build_config(
                tree_sitter_css::LANGUAGE.into(),
                "css",
                tree_sitter_css::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "javascript" | "js" | "jsx" => Some(JAVASCRIPT.get_or_init(|| {
            let highlights = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            );
            build_config(
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                &highlights,
                tree_sitter_javascript::INJECTIONS_QUERY,
                tree_sitter_javascript::LOCALS_QUERY,
            )
        })),
        "typescript" | "ts" => Some(TYPESCRIPT.get_or_init(|| {
            let highlights = format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            build_config(
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                &highlights,
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        })),
        "tsx" => Some(TSX.get_or_init(|| {
            let highlights = format!(
                "{}\n{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
                tree_sitter_typescript::HIGHLIGHTS_QUERY
            );
            build_config(
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                &highlights,
                "",
                tree_sitter_typescript::LOCALS_QUERY,
            )
        })),
        "python" | "py" => Some(PYTHON.get_or_init(|| {
            build_config(
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "c" => Some(C.get_or_init(|| {
            build_config(
                tree_sitter_c::LANGUAGE.into(),
                "c",
                tree_sitter_c::HIGHLIGHT_QUERY,
                "",
                "",
            )
        })),
        "rust" | "rs" => Some(RUST.get_or_init(|| {
            build_config(
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "zig" => Some(ZIG.get_or_init(|| {
            build_config(
                tree_sitter_zig::LANGUAGE.into(),
                "zig",
                tree_sitter_zig::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "make" | "makefile" | "gnumake" => Some(MAKE.get_or_init(|| {
            build_config(
                tree_sitter_make::LANGUAGE.into(),
                "make",
                tree_sitter_make::HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "xml" => Some(XML.get_or_init(|| {
            build_config(
                tree_sitter_xml::LANGUAGE_XML.into(),
                "xml",
                tree_sitter_xml::XML_HIGHLIGHT_QUERY,
                "",
                "",
            )
        })),
        "kotlin" | "kt" | "kts" => Some(KOTLIN.get_or_init(|| {
            build_config(
                tree_sitter_kotlin_ng::LANGUAGE.into(),
                "kotlin",
                KOTLIN_HIGHLIGHTS_QUERY,
                "",
                "",
            )
        })),
        "dart" => Some(DART.get_or_init(|| {
            build_config(
                tree_sitter_dart::LANGUAGE.into(),
                "dart",
                tree_sitter_dart::HIGHLIGHTS_QUERY,
                "",
                tree_sitter_dart::LOCALS_QUERY,
            )
        })),
        _ => None,
    }
}

fn build_config(
    language: Language,
    name: &'static str,
    highlights_query: &str,
    injections_query: &str,
    locals_query: &str,
) -> HighlightConfiguration {
    let mut config = HighlightConfiguration::new(
        language,
        name,
        highlights_query,
        injections_query,
        locals_query,
    )
    .expect("tree-sitter highlight query should be valid");
    config.configure(HIGHLIGHT_NAMES);
    config
}

fn style_for_capture(capture: &str) -> StyleKind {
    let root = capture.split('.').next().unwrap_or(capture);
    match root {
        "comment" => StyleKind::SyntaxComment,
        "string" | "character" | "escape" => StyleKind::SyntaxString,
        "number" | "boolean" => StyleKind::SyntaxNumber,
        "keyword" => StyleKind::SyntaxKeyword,
        "type" | "constructor" | "tag" => StyleKind::SyntaxType,
        "function" => StyleKind::SyntaxFunction,
        "property" | "attribute" => StyleKind::SyntaxProperty,
        "constant" => StyleKind::SyntaxConstant,
        "operator" => StyleKind::SyntaxOperator,
        _ => StyleKind::Code,
    }
}

fn push_span(spans: &mut Vec<StyleSpan>, start: usize, end: usize, kind: StyleKind) {
    if let Some(previous) = spans.last_mut()
        && previous.range.end == start
        && previous.kind == kind
    {
        previous.range.end = end;
    } else {
        spans.push(StyleSpan {
            range: start..end,
            kind,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_keywords_and_strings() {
        let spans = highlight("rust", "fn main() { println!(\"hello\"); }").unwrap();
        assert!(
            spans
                .iter()
                .any(|span| span.kind == StyleKind::SyntaxKeyword)
        );
        assert!(
            spans
                .iter()
                .any(|span| span.kind == StyleKind::SyntaxString)
        );
    }

    #[test]
    fn accepts_language_aliases() {
        assert!(highlight("pwsh", "Write-Host \"hello\"").is_some());
        assert!(highlight("c#", "class Example {}").is_some());
        assert!(highlight("Makefile", "all:\n\techo done").is_some());
    }

    #[test]
    fn supports_all_requested_languages() {
        let samples = [
            ("powershell", "$name = 'world'"),
            ("bash", "name=world"),
            ("fish", "set name world"),
            ("toml", "name = 'world'"),
            ("json", r#"{"name":"world"}"#),
            ("yaml", "name: world"),
            ("java", "class Example {}"),
            ("csharp", "class Example {}"),
            ("html", "<p>Hello</p>"),
            ("css", "p { color: red; }"),
            ("javascript", "const name = 'world';"),
            ("typescript", "const name: string = 'world';"),
            ("python", "name = 'world'"),
            ("c", "int main(void) { return 0; }"),
            ("rust", "fn main() {}"),
            ("zig", "pub fn main() void {}"),
            ("makefile", "all:\n\techo done"),
            ("xml", "<note>Hello</note>"),
            ("kotlin", "fun main() = println(\"hello\")"),
            ("dart", "void main() {}"),
        ];

        for (language, source) in samples {
            assert!(
                highlight(language, source).is_some(),
                "failed to configure {language}"
            );
        }
    }

    #[test]
    fn rejects_unknown_languages() {
        assert!(highlight("unknown-language", "some code").is_none());
    }
}
