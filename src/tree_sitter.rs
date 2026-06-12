use crossterm::style::Color;
use tree_sitter::{Language, Node, Parser};

pub struct TreeSitter {
    parser: Option<Parser>,
    pub highlights: Vec<(usize, usize, usize, u32, Color)>,
}

impl TreeSitter {
    pub fn new(filepath: Option<&str>) -> Self {
        let mut parser = Parser::new();
        if let Some(lang) = Self::detect_language(filepath) {
            parser.set_language(&lang).unwrap();
        } else {
            let default_lang: Language = tree_sitter_rust::LANGUAGE.into();
            parser.set_language(&default_lang).unwrap();
        }
        Self {
            parser: Some(parser),
            highlights: Vec::new(),
        }
    }

    pub fn reparse(&mut self, rows: &[String]) {
        let parser = match self.parser.as_mut() {
            Some(p) => p,
            None => return,
        };
        let source = rows.join("\n");
        if let Some(tree) = parser.parse(&source, None) {
            self.highlights.clear();
            self.collect_spans(tree.root_node(), 0, rows);
            self.highlights.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.3.cmp(&a.3)));
        }
    }

    fn collect_spans(&mut self, node: Node, depth: u32, rows: &[String]) {
        let kind = node.kind();
        let is_named = node.is_named();
        let start = node.start_position();
        let end = node.end_position();

        if let Some(color) = color_for_kind(kind, is_named) {
            if start.row == end.row {
                self.highlights.push((
                    start.row,
                    start.column,
                    end.column,
                    depth,
                    color,
                ));
            } else {
                let line_len = rows[start.row].len();
                self.highlights.push((
                    start.row,
                    start.column,
                    line_len,
                    depth,
                    color,
                ));
                for r in (start.row + 1)..end.row {
                    let len = rows[r].len();
                    self.highlights.push((r, 0, len, depth, color));
                }
                self.highlights.push((end.row, 0, end.column, depth, color));
            }
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                self.collect_spans(child, depth + 1, rows);
            }
        }
    }

    fn detect_language(filepath: Option<&str>) -> Option<Language> {
        let ext = filepath?.rsplit('.').next()?;
        match ext {
            "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
            "js" | "mjs" | "cjs" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
            "mts" | "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
            "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
            _ => None,
        }
    }
}

fn color_for_kind(kind: &str, is_named: bool) -> Option<Color> {
    if !is_named {
        match kind {
            "let" | "mut" | "fn" | "if" | "else" | "for" | "while" | "loop"
            | "match" | "return" | "struct" | "enum" | "impl" | "trait"
            | "pub" | "use" | "mod" | "in" | "ref" | "break" | "continue"
            | "as" | "where" | "type" | "const" | "static" | "unsafe"
            | "async" | "await" | "move" | "dyn" | "true" | "false"
            | "super" | "self" | "crate" | "extern" | "union" | "default"
            | "macro_rules"
            | "var" | "function" | "class" | "extends" | "implements"
            | "interface" | "new" | "this" | "do" | "switch" | "case"
            | "try" | "catch" | "finally" | "throw" | "import" | "export"
            | "from" | "of" | "yield" | "get" | "set" | "typeof"
            | "instanceof" | "void" | "delete" | "with" | "debugger"
            | "null" | "undefined" | "any" | "never" | "unknown"
            | "abstract" | "private" | "protected" | "public" | "readonly"
            | "declare" | "keyof" | "infer" | "satisfies" | "asserts"
            | "module" | "namespace" | "global" | "require" => Some(Color::AnsiValue(33)),
            _ => None,
        }
    } else {
        match kind {
            "string_literal" | "raw_string_literal" | "string" | "template_string"
            | "escape_sequence" | "char_literal" | "regex" | "regex_pattern" => Some(Color::AnsiValue(28)),
            "line_comment" | "block_comment" | "comment" => Some(Color::AnsiValue(245)),
            "integer_literal" | "float_literal" | "number" => Some(Color::AnsiValue(198)),
            "type_identifier" | "primitive_type" | "predefined_type" => Some(Color::AnsiValue(44)),
            "self" | "super" | "crate" | "this" => Some(Color::AnsiValue(33)),
            _ => None,
        }
    }
}
