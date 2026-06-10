use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{read, Event, KeyCode, KeyEventKind},
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
}

impl Editor {
    fn new(treesitter_enabled: bool) -> Self {
        Self {
            cursor_x: 0,
            cursor_y: 0,
            row_offset: 0,
            col_offset: 0,
            rows: vec![
                "Nano-rs".to_string(),
                "".to_string(),
            ],
            should_quit: false,
            filepath: None,
            dirty: false,

            treesitter: treesitter_enabled.then(|| TreeSitter::new(None)),
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
        write!(
            out,
            "{:width$}",
            format!(
                "[Ctrl+Q quit] Ln {}, Col {}",
                self.cursor_y + 1,
                self.cursor_col() + 1
            ),
            width = width as usize,
        )?;
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

                match key.code {
                    KeyCode::Char('q')
                        if key.modifiers.contains(
                            crossterm::event::KeyModifiers::CONTROL
                        ) =>
                    {
                        self.should_quit = true;
                    }

                    KeyCode::Char('s')
                        if key.modifiers.contains(
                            crossterm::event::KeyModifiers::CONTROL
                        ) =>
                    {
                        self.dirty = false;
                        if let Some(path) = &self.filepath {
                            std::fs::write(path, self.rows.join("\n"))?;
                            if let Some(ts) = &mut self.treesitter {
                                ts.reparse(&self.rows);
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

                    KeyCode::Backspace => {
                        if self.cursor_x > 0 {
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
                            let current = self.rows.remove(self.cursor_y);
                            self.cursor_y -= 1;
                            self.cursor_x = self.rows[self.cursor_y].len();
                            self.rows[self.cursor_y].push_str(&current);
                            self.dirty = true;
                        }
                    }

                    KeyCode::Char(c) => {
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

    let filepath: Option<String> = noargs::arg("[filepath]")
        .take(&mut args).present_and_then(|a| a.value().parse())?;

    if let Some(help) = args.finish()? {
        // When help is requested, finish() returns the built help text.
        // Print it here and exit without running application logic.
        print!("{help}");
        return Ok(());
    }

    let config = load_config(configpath).unwrap_or_else(|e| {
        eprintln!("Warning: failed to load config: {e}, using defaults");
        Config { treesitter: true }
    });

    let mut editor = Editor::new(config.treesitter);
    editor.run(filepath).unwrap_or_else(|e| {
        println!("Error: {e}");
    });

    Ok(())
}
