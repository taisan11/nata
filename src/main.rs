use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{read, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Attribute, ResetColor, SetAttribute, SetForegroundColor},
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
mod tree_sitter;
use tree_sitter::TreeSitter;
mod config;
use config::{load_config,Config};
// use env_logger;
// use log::{error, warn, info, debug};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone)]
struct UndoRecord {
    rows: Vec<String>,
    cursor_x: usize,
    cursor_y: usize,
}

enum AskKind {
    FileNotFound,
}

enum EditorMode {
    Normal,
    AskCreate(AskKind),
    EnterFilename(String),
    ConfirmQuit,
}

struct Editor {
    cursor_x: usize,
    cursor_y: usize,

    row_offset: usize, // スクロール位置
    col_offset: usize,

    rows: Vec<String>,

    should_quit: bool,

    filepath: Option<String>,

    dirty: bool,

    treesitter: Option<TreeSitter>,

    undo_stack: Vec<UndoRecord>,
    redo_stack: Vec<UndoRecord>,

    mode: EditorMode,
    key_modifier: KeyModifiers,
}

impl Editor {
    fn new(treesitter_enabled: bool, key_modifier: KeyModifiers) -> Self {
        Self {
            cursor_x: 0,
            cursor_y: 0,
            row_offset: 0,
            col_offset: 0,
            rows: vec![
                "".to_string(),
            ],
            should_quit: false,
            filepath: None,
            dirty: false,

            treesitter: treesitter_enabled.then(|| TreeSitter::new(None)),

            undo_stack: Vec::new(),
            redo_stack: Vec::new(),

            mode: EditorMode::Normal,
            key_modifier,
        }
    }

    fn run(&mut self,filepath: Option<String>) -> Result<()> {
        self.filepath = filepath;
        let _terminal = TerminalGuard::new()?;

        if let Some(path) = &self.filepath {
            if let Ok(contents) = std::fs::read_to_string(path) {
                self.rows = contents.lines().map(|l| l.to_string()).collect();
            }
            self.dirty = false;
        }

        if let Some(ts) = &mut self.treesitter {
            *ts = TreeSitter::new(self.filepath.as_deref());
            ts.reparse(&self.rows);
        }

        while !self.should_quit {
            self.draw()?;
            self.process_keypress()?;
        }
        // debug!("Exiting editor");

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
                "nata - {} lines - Open: {}{}",
                self.rows.len(),
                self.filepath.as_deref().unwrap_or("newfile"),
                if self.dirty { " (*)" } else { "" }
            ),
            width = width as usize,
        )?;
        queue!(out, SetAttribute(Attribute::Reset))?;

        let text_height = height.saturating_sub(2);

        for y in 0..text_height {
            queue!(out, MoveTo(0, y+1))?;

            let row_index = self.row_offset + y as usize;
            if let Some(row) = self.rows.get(row_index) {
                let start = col_to_byte(row, self.col_offset);
                let end_vis = self.col_offset.saturating_add(width as usize);
                let mut end = col_to_byte(row, end_vis);
                if end == start && start < row.len() {
                    end = start + row[start..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                }
                let colored = self.color_slice(row, row_index, start, end);
                write!(out, "{colored}")?;
            }
        }

        // ステータスバー
        queue!(out, MoveTo(0, height - 1))?;
        queue!(out, SetAttribute(Attribute::Reverse))?;
        match &self.mode {
            EditorMode::Normal => {
                write!(
                    out,
                    "{:width$}",
                    format!(
                        "[Ctrl+Q quit] [Ctrl+S save] [Ctrl+Z/Y undo/redo] Ln {}, Col {}",
                        self.cursor_y + 1,
                        self.cursor_col() + 1
                    ),
                    width = width as usize,
                )?;
            }
            EditorMode::AskCreate(kind) => {
                let msg = match kind {
                    AskKind::FileNotFound => format!(
                        "File '{}' not found. Create? (y/n)",
                        self.filepath.as_deref().unwrap_or("")
                    ),
                };
                write!(out, "{:width$}", msg, width = width as usize)?;
            }
            EditorMode::EnterFilename(buffer) => {
                let msg = format!("Filename: {}", buffer);
                write!(out, "{:width$}", msg, width = width as usize)?;
            }
            EditorMode::ConfirmQuit => {
                write!(
                    out,
                    "{:width$}",
                    "Unsaved changes. Quit without saving? (y/n)",
                    width = width as usize,
                )?;
            }
        }
        queue!(out, SetAttribute(Attribute::Reset))?;

        queue!(
            out,
            MoveTo(
                (self.cursor_col() - self.col_offset) as u16,
                (self.cursor_y - self.row_offset + 1) as u16
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

        // horizontal (visual columns)
        let cur_col = self.cursor_col();
        if cur_col < self.col_offset {
            self.col_offset = cur_col;
        } else if cur_col >= self.col_offset.saturating_add(width as usize) {
            self.col_offset = cur_col - width as usize + 1;
        }

        Ok(())
    }

    fn process_keypress(&mut self) -> Result<()> {
        loop {
            let event = match read() {
                Ok(e) => e,
                Err(_) => continue,
            };
            if let Event::Key(key) = event {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match &mut self.mode {
                    EditorMode::AskCreate(kind) => {
                        match key.code {
                            KeyCode::Char('y' | 'Y') => {
                                match kind {
                                    AskKind::FileNotFound => {
                                        if let Some(path) = &self.filepath {
                                            std::fs::write(path, self.rows.join("\n"))?;
                                            self.dirty = false;
                                            if let Some(ts) = &mut self.treesitter {
                                                *ts = TreeSitter::new(self.filepath.as_deref());
                                                ts.reparse(&self.rows);
                                            }
                                        }
                                        self.mode = EditorMode::Normal;
                                    }
                                }
                            }
                            KeyCode::Char('n' | 'N') => {
                                self.mode = EditorMode::Normal;
                            }
                            KeyCode::Char('q')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.should_quit = true;
                            }
                            _ => {}
                        }
                        break;
                    }

                    EditorMode::EnterFilename(buffer) => {
                        match key.code {
                            KeyCode::Char('q')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.should_quit = true;
                            }
                            KeyCode::Enter if !buffer.is_empty() => {
                                let path = std::mem::take(buffer);
                                std::fs::write(&path, self.rows.join("\n"))?;
                                self.filepath = Some(path);
                                self.dirty = false;
                                if let Some(ts) = &mut self.treesitter {
                                    *ts = TreeSitter::new(self.filepath.as_deref());
                                    ts.reparse(&self.rows);
                                }
                                self.mode = EditorMode::Normal;
                            }
                            KeyCode::Backspace => {
                                buffer.pop();
                            }
                            KeyCode::Char(c) => {
                                buffer.push(c);
                            }
                            _ => {}
                        }
                        break;
                    }

                    EditorMode::ConfirmQuit => {
                        match key.code {
                            KeyCode::Char('y' | 'Y') => {
                                self.should_quit = true;
                            }
                            KeyCode::Char('n' | 'N') => {
                                self.mode = EditorMode::Normal;
                            }
                            KeyCode::Char('q')
                                if key.modifiers.contains(self.key_modifier) =>
                            {
                                self.should_quit = true;
                            }
                            _ => {}
                        }
                        break;
                    }

                    EditorMode::Normal => {
                        match key.code {
                            KeyCode::Char('q')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                if self.dirty {
                                    self.mode = EditorMode::ConfirmQuit;
                                } else {
                                    self.should_quit = true;
                                }
                            }

                            KeyCode::Char('s')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                match &self.filepath {
                                    Some(path) if std::fs::metadata(path).is_ok() => {
                                        std::fs::write(path, self.rows.join("\n"))?;
                                        self.dirty = false;
                                        if let Some(ts) = &mut self.treesitter {
                                            ts.reparse(&self.rows);
                                        }
                                    }
                                    Some(_) => {
                                        self.mode = EditorMode::AskCreate(AskKind::FileNotFound);
                                    }
                                    None => {
                                        self.mode = EditorMode::EnterFilename(String::new());
                                    }
                                }
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
                                } else if self.cursor_x > 0 {
                                    let row = &self.rows[self.cursor_y];
                                    self.cursor_x = row[..self.cursor_x]
                                        .char_indices()
                                        .last()
                                        .map(|(i, _)| i)
                                        .unwrap();
                                }
                            }

                            KeyCode::Right => {
                                let row = self.rows
                                    .get(self.cursor_y)
                                    .map(|r| r.as_str())
                                    .unwrap_or("");
                                let row_len = row.len();

                                if self.cursor_x >= row_len
                                    && self.cursor_y + 1
                                        < self.rows.len()
                                {
                                    self.cursor_y += 1;
                                    self.cursor_x = 0;
                                } else if self.cursor_x < row_len {
                                    self.cursor_x += row[self.cursor_x..]
                                        .chars()
                                        .next()
                                        .map(|c| c.len_utf8())
                                        .unwrap_or(1);
                                }
                            }

                            KeyCode::Enter => {
                                self.save_undo_state();
                                let rest = self.rows[self.cursor_y]
                                    .split_off(self.cursor_x);
                                self.rows.insert(
                                    self.cursor_y + 1,
                                    rest,
                                );

                                self.cursor_y += 1;
                                self.cursor_x = 0;
                                self.dirty = true;
                            }

                            KeyCode::Backspace
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.delete_word_backward();
                            }

                            // Ctrl+H fallback for terminals where Ctrl+Backspace sends \x08
                            KeyCode::Char('h')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.delete_word_backward();
                            }

                            KeyCode::Backspace => {
                                if self.cursor_x > 0 {
                                    self.save_undo_state();
                                    let row =
                                        &mut self.rows[self.cursor_y];
                                    let prev = row[..self.cursor_x]
                                        .char_indices()
                                        .last()
                                        .map(|(i, _)| i)
                                        .unwrap();
                                    row.remove(prev);

                                    self.cursor_x = prev;
                                    self.dirty = true;
                                } else if self.cursor_y > 0 {
                                    self.save_undo_state();
                                    let current = self.rows.remove(self.cursor_y);
                                    self.cursor_y -= 1;
                                    self.cursor_x = self.rows[self.cursor_y].len();
                                    self.rows[self.cursor_y].push_str(&current);
                                    self.dirty = true;
                                }
                            }

                            KeyCode::Char('z')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.undo();
                            }

                            KeyCode::Char('y')
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.redo();
                            }

                            KeyCode::Delete
                                if key.modifiers.contains(
                                    self.key_modifier
                                ) =>
                            {
                                self.delete_word_forward();
                            }

                            KeyCode::Delete => {
                                let row_len = self.rows[self.cursor_y].len();
                                if self.cursor_x < row_len {
                                    self.save_undo_state();
                                    let row = &mut self.rows[self.cursor_y];
                                    let next = row[self.cursor_x..]
                                        .chars()
                                        .next()
                                        .map(|c| c.len_utf8())
                                        .unwrap_or(1);
                                    row.drain(self.cursor_x..self.cursor_x + next);
                                    self.dirty = true;
                                } else if self.cursor_y + 1 < self.rows.len() {
                                    self.save_undo_state();
                                    let next = self.rows.remove(self.cursor_y + 1);
                                    self.rows[self.cursor_y].push_str(&next);
                                    self.dirty = true;
                                }
                            }

                            KeyCode::Char(c) => {
                                self.save_undo_state();
                                let row =
                                    &mut self.rows[self.cursor_y];

                                row.insert(self.cursor_x, c);

                                self.cursor_x += c.len_utf8();
                                self.dirty = true;
                            }

                            _ => {}
                        }

                        if let Some(ts) = &mut self.treesitter {
                            ts.reparse(&self.rows);
                        }
                        self.scroll()?;
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    fn save_undo_state(&mut self) {
        self.undo_stack.push(UndoRecord {
            rows: self.rows.clone(),
            cursor_x: self.cursor_x,
            cursor_y: self.cursor_y,
        });
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        if let Some(record) = self.undo_stack.pop() {
            self.redo_stack.push(UndoRecord {
                rows: self.rows.clone(),
                cursor_x: self.cursor_x,
                cursor_y: self.cursor_y,
            });
            self.rows = record.rows;
            self.cursor_x = record.cursor_x;
            self.cursor_y = record.cursor_y;
            self.dirty = true;
        }
    }

    fn redo(&mut self) {
        if let Some(record) = self.redo_stack.pop() {
            self.undo_stack.push(UndoRecord {
                rows: self.rows.clone(),
                cursor_x: self.cursor_x,
                cursor_y: self.cursor_y,
            });
            self.rows = record.rows;
            self.cursor_x = record.cursor_x;
            self.cursor_y = record.cursor_y;
            self.dirty = true;
        }
    }

    fn delete_word_backward(&mut self) {
        if self.cursor_x == 0 {
            if self.cursor_y > 0 {
                self.save_undo_state();
                let current = self.rows.remove(self.cursor_y);
                self.cursor_y -= 1;
                self.cursor_x = self.rows[self.cursor_y].len();
                self.rows[self.cursor_y].push_str(&current);
                self.dirty = true;
            }
            return;
        }

        let row = self.rows[self.cursor_y].clone();
        let before = &row[..self.cursor_x];
        let mut start = self.cursor_x;
        let mut rev_chars: Vec<char> = before.chars().collect();

        while let Some(&c) = rev_chars.last() {
            if is_word_delimiter(c) {
                start -= c.len_utf8();
                rev_chars.pop();
            } else {
                break;
            }
        }

        while let Some(&c) = rev_chars.last() {
            if !is_word_delimiter(c) {
                start -= c.len_utf8();
                rev_chars.pop();
            } else {
                break;
            }
        }

        self.save_undo_state();
        let row = &mut self.rows[self.cursor_y];
        row.drain(start..self.cursor_x);
        self.cursor_x = start;
        self.dirty = true;
    }

    fn delete_word_forward(&mut self) {
        let row_len = self.rows.get(self.cursor_y).map(|r| r.len()).unwrap_or(0);
        if self.cursor_x >= row_len {
            if self.cursor_y + 1 < self.rows.len() {
                self.save_undo_state();
                let next = self.rows.remove(self.cursor_y + 1);
                self.rows[self.cursor_y].push_str(&next);
                self.dirty = true;
            }
            return;
        }

        let row = self.rows[self.cursor_y].clone();
        let rest = &row[self.cursor_x..];
        let mut end = self.cursor_x;
        let mut chars = rest.chars();

        for c in &mut chars {
            if is_word_delimiter(c) {
                end += c.len_utf8();
            } else {
                end += c.len_utf8();
                break;
            }
        }

        for c in &mut chars {
            if is_word_delimiter(c) {
                break;
            }
            end += c.len_utf8();
        }

        self.save_undo_state();
        let row = &mut self.rows[self.cursor_y];
        row.drain(self.cursor_x..end);
        self.dirty = true;
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

    fn color_slice(
        &self,
        line: &str,
        row_index: usize,
        start: usize,
        end: usize,
    ) -> String {
        let ts = match &self.treesitter {
            Some(ts) => ts,
            None => return line[start..end].to_string(),
        };

        let mut colored = String::new();
        let mut idx = start;

        for &(r, s, e, _, color) in &ts.highlights {
            if r != row_index || e <= start || s >= end {
                continue;
            }

            if s >= idx {
                if idx < s && idx < end {
                    colored.push_str(&line[idx..s.min(end)]);
                }

                if s < end {
                    colored.push_str(&format!("{}", SetForegroundColor(color)));
                    colored.push_str(&line[s..e.min(end)]);
                    colored.push_str(&format!("{}", ResetColor));
                }

                idx = e.max(idx);
            }

            if idx >= end {
                break;
            }
        }

        if idx < end {
            colored.push_str(&line[idx..end]);
        }

        colored
    }

    fn cursor_col(&self) -> usize {
        self.rows
            .get(self.cursor_y)
            .map(|row| byte_to_col(row, self.cursor_x))
            .unwrap_or(0)
    }
}

fn byte_to_col(row: &str, byte: usize) -> usize {
    row[..byte].width()
}

fn col_to_byte(row: &str, col: usize) -> usize {
    let mut vis = 0;
    for (i, c) in row.char_indices() {
        let w = c.width().unwrap_or(0);
        if vis + w > col {
            return i;
        }
        vis += w;
    }
    row.len()
}

fn is_word_delimiter(c: char) -> bool {
    matches!(c,'　' | ' ' | ',' | ':' | '。' | '、' | '?' | '!'|'！'|'？')
}

fn default_config_path() -> String {
    if let Ok(home) = std::env::var("HOME") {
        std::path::PathBuf::from(home)
            .join(".config")
            .join("nata")
            .join("config.json")
            .to_string_lossy()
            .into_owned()
    } else {
        "config.json".to_string()
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
    // let file = std::fs::OpenOptions::new()
    //     .create(true)
    //     .append(true)
    //     .open("app.log")
    //     .unwrap();

    // // env_logger のBuilderを初期化し、ターゲットをファイルに変更
    // env_logger::Builder::new()
    //     .target(env_logger::Target::Pipe(Box::new(file)))
    //     .filter_level(log::LevelFilter::Debug) // デフォルトでDebug以上を出力
    //     .parse_default_env() // RUST_LOG で上書き可能
    //     .init();
    let mut args = noargs::raw_args();
    args.metadata_mut().app_name = env!("CARGO_PKG_NAME");
    args.metadata_mut().app_description = env!("CARGO_PKG_DESCRIPTION");

    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    noargs::HELP_FLAG.take_help(&mut args);

    let configpath: Option<String> = noargs::opt("config")
        .short('c')
        .take(&mut args).present_and_then(|o| o.value().parse())?;
    let configpath = configpath.unwrap_or_else(default_config_path);

    let filepath: Option<String> = noargs::arg("[filepath]")
        .take(&mut args).present_and_then(|a| a.value().parse())?;

    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(());
    }

    let config = if std::fs::metadata(&configpath).is_ok() {
        load_config(Some(configpath.clone())).unwrap_or_else(|e| {
            eprintln!("Warning: failed to load config: {e}, using defaults");
            Config { treesitter: true, key_modifier: "ctrl".to_string() }
        })
    } else {
        if let Some(parent) = std::path::Path::new(&configpath).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&configpath, "{\n    \"treesitter\": true,\n    \"key_modifier\": \"ctrl\"\n}\n") {
            eprintln!("Warning: failed to create default config: {e}");
        }
        Config { treesitter: true, key_modifier: "ctrl".to_string() }
    };

    let modifier = match config.key_modifier.to_lowercase().as_str() {
        "super" => KeyModifiers::SUPER,
        _ => KeyModifiers::CONTROL,
    };

    let mut editor = Editor::new(config.treesitter, modifier);
    editor.run(filepath).unwrap_or_else(|e| {
        println!("Error: {e}");
    });

    Ok(())
}
