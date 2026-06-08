use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{read, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Attribute, SetAttribute},
    terminal::{
        disable_raw_mode,
        enable_raw_mode,
        size,
        Clear,
        ClearType,
        EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use std::io::{stdout, Write};
use tree_sitter::{Language, Node, Parser};
use tree_sitter_rust;

const C_KW: &str = "\x1b[38;5;33m";
const C_STR: &str = "\x1b[38;5;28m";
const C_CMT: &str = "\x1b[38;5;245m";
const C_NUM: &str = "\x1b[38;5;198m";
const C_TYP: &str = "\x1b[38;5;44m";
const C_RS: &str = "\x1b[0m";

struct Editor {
    cursor_x: usize,
    cursor_y: usize,

    row_offset: usize, // スクロール位置
    col_offset: usize,

    rows: Vec<String>,

    should_quit: bool,

    filepath: Option<String>,

    lang: Option<Language>,
    parser: Option<Parser>,
    highlights: Vec<(usize, usize, usize, u32, &'static str)>,
}

impl Editor {
    fn new() -> Self {
        Self {
            cursor_x: 0,
            cursor_y: 1,
            row_offset: 0,
            col_offset: 0,
            rows: vec![
                "Nano-rs".to_string(),
                "".to_string(),
            ],
            should_quit: false,
            filepath: None,

            lang: None,
            parser: None,
            highlights: Vec::new(),
        }
    }

    fn run(&mut self,filepath: Option<String>) -> Result<()> {
        self.filepath = filepath;
        let _terminal = TerminalGuard::new()?;

        if let Some(path) = &self.filepath {
            if let Ok(contents) = std::fs::read_to_string(path) {
                self.rows = contents.lines().map(|l| l.to_string()).collect();
            }
        }

        self.init_treesitter();

        while !self.should_quit {
            self.draw()?;
            self.process_keypress()?;
        }

        Ok(())
    }

    fn draw(&self) -> Result<()> {
        let mut out = stdout();
        let (width, height) = size()?;

        queue!(
            out,
            Hide,
            MoveTo(0, 0),
            Clear(ClearType::All)
        )?;

        queue!(
            out,
            MoveTo(0,0)
        )?;

        // ヘッダ
        queue!(out, SetAttribute(Attribute::Reverse))?;
        write!(
            out,
            "{:width$}",
            format!(
                "Nano-rs - {} lines - Open: {}",
                self.rows.len()-1,
                self.filepath.as_deref().unwrap_or("newfile")
            ),
            width = width as usize,
        )?;
        queue!(out, SetAttribute(Attribute::Reset))?;

        let text_height = height.saturating_sub(2);

        for y in 0..text_height {
            queue!(out, MoveTo(0, y+1))?;

            let row_index = self.row_offset + y as usize;
            if let Some(row) = self.rows.get(row_index) {
                let start = self.col_offset.min(row.len());
                let end = (self.col_offset + width as usize).min(row.len());
                let colored = self.color_slice(row, row_index, start, end);
                write!(out, "{colored}")?;
            }
        }

        // ステータスバー
        queue!(out, MoveTo(0, height - 1))?;
        queue!(out, SetAttribute(Attribute::Reverse))?;
        write!(
            out,
            "{:width$}",
            format!(
                "[Ctrl+Q quit] Ln {}, Col {}",
                self.cursor_y,
                self.cursor_x + 1
            ),
            width = width as usize,
        )?;
        queue!(out, SetAttribute(Attribute::Reset))?;

        queue!(
            out,
            MoveTo(
                (self.cursor_x - self.col_offset) as u16,
                (self.cursor_y - self.row_offset) as u16
            ),
            Show
        )?;

        out.flush()?;

        Ok(())
    }

    fn scroll(&mut self) -> Result<()> {
        let (width, height) = size()?;
        let text_height = height.saturating_sub(1);

        // vertical
        if self.cursor_y < self.row_offset {
            self.row_offset = self.cursor_y;
        } else if self.cursor_y >= self.row_offset + text_height as usize {
            self.row_offset = self.cursor_y - text_height as usize + 1;
        }

        // horizontal
        if self.cursor_x < self.col_offset {
            self.col_offset = self.cursor_x;
        } else if self.cursor_x >= self.col_offset + width as usize {
            self.col_offset = self.cursor_x - width as usize + 1;
        }

        Ok(())
    }

    fn process_keypress(&mut self) -> Result<()> {
        loop {
            if let Event::Key(key) = read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q')
                        if key.modifiers.contains(
                            crossterm::event::KeyModifiers::CONTROL
                        ) =>
                    {
                        self.should_quit = true;
                    }

                    KeyCode::Up => {
                        if self.cursor_y > 0 {
                            self.cursor_y =
                                self.cursor_y.saturating_sub(1);
                            self.clamp_cursor_x();
                        }
                    }

                    KeyCode::Down => {
                        self.cursor_y += 1;

                        if self.cursor_y >= self.rows.len() {
                            self.cursor_y =
                                self.rows.len() - 1;
                        }
                        self.clamp_cursor_x();
                    }

                    KeyCode::Left => {
                        if self.cursor_x == 0
                            && self.cursor_y > 0
                        {
                            self.cursor_y -= 1;
                            self.cursor_x = self.rows
                                [self.cursor_y]
                                .len();
                        } else {
                            self.cursor_x = self
                                .cursor_x
                                .saturating_sub(1);
                        }
                    }

                    KeyCode::Right => {
                        let row_len = self
                            .rows
                            .get(self.cursor_y)
                            .map(|r| r.len())
                            .unwrap_or(0);

                        if self.cursor_x >= row_len
                            && self.cursor_y + 1
                                < self.rows.len()
                        {
                            self.cursor_y += 1;
                            self.cursor_x = 0;
                        } else {
                            self.cursor_x = (self
                                .cursor_x + 1)
                                .min(row_len);
                        }
                    }

                    KeyCode::Enter => {
                        self.rows.insert(
                            self.cursor_y + 1,
                            String::new(),
                        );

                        self.cursor_y += 1;
                        self.cursor_x = 0;
                    }

                    KeyCode::Backspace => {
                        if self.cursor_x > 0 {
                            let row =
                                &mut self.rows[self.cursor_y];

                            row.remove(self.cursor_x - 1);

                            self.cursor_x -= 1;
                        }
                    }

                    KeyCode::Char(c) => {
                        let row =
                            &mut self.rows[self.cursor_y];

                        row.insert(self.cursor_x, c);

                        self.cursor_x += 1;
                    }

                    _ => {}
                }

                self.reparse();
                self.scroll()?;
                break;
            }
        }

        Ok(())
    }

    fn clamp_cursor_x(&mut self) {
        let row_len = self
            .rows
            .get(self.cursor_y)
            .map(|r| r.len())
            .unwrap_or(0);
        if self.cursor_x > row_len {
            self.cursor_x = row_len;
        }
    }

    fn init_treesitter(&mut self) {
        let lang = self.detect_language();
        if let Some(lang) = lang {
            let mut parser = Parser::new();
            if parser.set_language(&lang).is_ok() {
                self.lang = Some(lang);
                self.parser = Some(parser);
                self.reparse();
            }
        }
    }

    fn detect_language(&self) -> Option<Language> {
        let path = self.filepath.as_deref()?;
        if path.ends_with(".rs") {
            Some(tree_sitter_rust::LANGUAGE.into())
        } else {
            None
        }
    }

    fn reparse(&mut self) {
        let parser = match self.parser.as_mut() {
            Some(p) => p,
            None => return,
        };
        let source = self.rows.join("\n");
        if let Some(tree) = parser.parse(&source, None) {
            self.highlights.clear();
            self.collect_spans(tree.root_node(), 0);
            self.highlights.sort_by(|a, b| b.3.cmp(&a.3));
        }
    }

    fn collect_spans(&mut self, node: Node, depth: u32) {
        let kind = node.kind();
        let is_named = node.is_named();
        let start = node.start_position();
        let end = node.end_position();

        if let Some(color) = color_for_kind(kind, is_named) {
            if start.row == end.row {
                self.highlights.push((start.row, start.column, end.column, depth, color));
            } else {
                let line_len = self.rows[start.row].len();
                self.highlights.push((start.row, start.column, line_len, depth, color));
                for r in (start.row + 1)..end.row {
                    let len = self.rows[r].len();
                    self.highlights.push((r, 0, len, depth, color));
                }
                self.highlights.push((end.row, 0, end.column, depth, color));
            }
        }

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                self.collect_spans(child, depth + 1);
            }
        }
    }

    fn color_slice(&self, row: &str, row_index: usize, start: usize, end: usize) -> String {
        let len = end - start;
        if self.highlights.is_empty() || len == 0 {
            return row[start..end].to_string();
        }

        let mut line_colors: Vec<Option<&str>> = vec![None; len];

        for &(hr, hs, he, _depth, color) in &self.highlights {
            if hr != row_index || he <= start || hs >= end {
                continue;
            }
            let lo = if hs > start { hs - start } else { 0 };
            let hi = if he < end { he - start } else { len };
            for i in lo..hi {
                if line_colors[i].is_none() {
                    line_colors[i] = Some(color);
                }
            }
        }

        let mut result = String::new();
        let mut i = 0;
        while i < len {
            if let Some(c) = line_colors[i] {
                result.push_str(c);
                let chunk_start = i;
                while i < len && line_colors[i] == Some(c) {
                    i += 1;
                }
                result.push_str(&row[start + chunk_start..start + i]);
                result.push_str(C_RS);
            } else {
                let chunk_start = i;
                while i < len && line_colors[i].is_none() {
                    i += 1;
                }
                result.push_str(&row[start + chunk_start..start + i]);
            }
        }

        result
    }
}

fn color_for_kind(kind: &str, is_named: bool) -> Option<&'static str> {
    if !is_named {
        match kind {
            "let"|"mut"|"fn"|"if"|"else"|"for"|"while"|"loop"|"match"|"return"
            |"struct"|"enum"|"impl"|"trait"|"pub"|"use"|"mod"|"in"|"ref"
            |"break"|"continue"|"as"|"where"|"type"|"const"|"static"|"unsafe"
            |"async"|"await"|"move"|"dyn"|"true"|"false"|"super"|"self"|"crate"
            |"extern"|"union"|"default"|"macro_rules" => Some(C_KW),
            _ => None,
        }
    } else {
        match kind {
            "string_literal"|"raw_string_literal" => Some(C_STR),
            "line_comment"|"block_comment" => Some(C_CMT),
            "integer_literal"|"float_literal" => Some(C_NUM),
            "escape_sequence"|"char_literal" => Some(C_STR),
            "type_identifier"|"primitive_type" => Some(C_TYP),
            "self"|"super"|"crate" => Some(C_KW),
            _ => None,
        }
    }
}

struct TerminalGuard;

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;

        execute!(
            stdout(),
            EnterAlternateScreen,
            Hide
        )?;

        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();

        let _ = execute!(
            stdout(),
            Show,
            LeaveAlternateScreen
        );
    }
}

fn main() -> noargs::Result<()> {
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = env!("CARGO_PKG_DESCRIPTION");

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    noargs::HELP_FLAG.take_help(&mut args);

    let filepath: Option<String> = noargs::arg("[filepath]")
        .take(&mut args).present_and_then(|a| a.value().parse())?;

    if let Some(help) = args.finish()? {
        // When help is requested, finish() returns the built help text.
        // Print it here and exit without running application logic.
        print!("{help}");
        return Ok(());
    }

    let mut editor = Editor::new();
    editor.run(filepath)?;

    Ok(())
}
