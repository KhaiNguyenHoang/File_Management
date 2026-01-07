use std::{collections::HashSet, path::PathBuf};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
};
use walkdir::WalkDir;

use crate::ops;

#[derive(Debug, Clone, PartialEq)]
pub enum ClipboardOp {
    Copy,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActiveFocus {
    FileList,
    Preview,
}

#[derive(Clone, Debug)]
pub enum PopupState {
    None,
    Chmod {
        path: PathBuf,
        mode: u32,
        cursor_idx: usize, // 0-8 for rwx * 3
    },
}

pub struct AppState {
    pub cwd: PathBuf,
    pub entries: Vec<FsEntry>,
    pub cursor: usize,
    pub selected: HashSet<PathBuf>,
    pub preview: PreviewState,
    pub syntax_set: SyntaxSet,
    pub theme_set: ThemeSet,
    pub clipboard: Option<(ClipboardOp, Vec<PathBuf>)>,

    // UI State
    pub active_focus: ActiveFocus,
    pub preview_scroll: usize,
    pub popup: PopupState,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("cwd", &self.cwd)
            .field("entries", &self.entries)
            .field("cursor", &self.cursor)
            .field("selected", &self.selected)
            .field("preview", &self.preview)
            .field("clipboard", &self.clipboard)
            .field("active_focus", &self.active_focus)
            .field("preview_scroll", &self.preview_scroll)
            .finish()
    }
}

#[derive(Debug)]
pub struct FsEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub _size: u64,
    pub permissions: String,
}

#[derive(Debug)]
pub enum PreviewState {
    None,
    Ready(PreviewContent),
    Loading { _path: PathBuf },
    Error { _path: PathBuf, message: String },
}

#[derive(Clone, Debug)]
pub enum PreviewContent {
    Text {
        title: String,
        content: String,
    },
    Binary {
        title: String,
        size: u64,
    },
    Image {
        title: String,
        width: u32,
        height: u32,
        color_type: String,
    },
}

#[derive(Clone, Debug)]
pub enum Action {
    CursorMoveUp,
    CursorMoveDown,
    RequestPreview(PathBuf),
    ToggleSelect,
    EnterDir,
    GoBack,
    PreviewReady(PreviewContent),
    PreviewError { path: PathBuf, error: String },
    Yank,
    Paste,
    Delete,
    Chmod, // Opens Popup
    Open,
    
    // Focus & Scroll
    SwitchFocus,
    ScrollPreviewUp,
    ScrollPreviewDown,
    ScrollPreviewPageUp,
    ScrollPreviewPageDown,

    // Popup Actions
    PopupUp,
    PopupDown,
    PopupLeft,
    PopupRight,
    PopupToggle,
    PopupSubmit,
    PopupCancel,
}

pub trait Reducer {
    fn reduce(&mut self, action: Action);
}

impl Reducer for AppState {
    fn reduce(&mut self, action: Action) {
        match action {
            Action::CursorMoveUp => {
                if self.active_focus == ActiveFocus::FileList {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
            }
            Action::CursorMoveDown => {
                if self.active_focus == ActiveFocus::FileList {
                    if self.cursor + 1 < self.entries.len() {
                        self.cursor += 1;
                    }
                }
            }
            Action::EnterDir => {
                let mut new_cwd = self.cwd.clone();
                if let Some(entry) = self.entries.get(self.cursor) {
                    if entry.is_dir {
                        new_cwd = entry.path.clone();
                    }
                }

                if new_cwd != self.cwd {
                    if let Ok(entries) = read_entries(&new_cwd) {
                        self.cwd = new_cwd;
                        self.entries = entries;
                        self.cursor = 0;
                        self.preview = PreviewState::None;
                        self.preview_scroll = 0;
                        // Keep focus on FileList or reset? Let's keep it.
                    }
                }
            }
            Action::GoBack => {
                if let Some(parent) = self.cwd.parent() {
                    let new_cwd = parent.to_path_buf();
                    if let Ok(entries) = read_entries(&new_cwd) {
                        self.cwd = new_cwd;
                        self.entries = entries;
                        self.cursor = 0;
                        self.preview = PreviewState::None;
                        self.preview_scroll = 0;
                    }
                }
            }
            Action::RequestPreview(path) => {
                self.preview = PreviewState::Loading { _path: path };
                self.preview_scroll = 0;
            }
            Action::ToggleSelect => {
                if let Some(entry) = self.entries.get(self.cursor) {
                    let path = entry.path.clone();
                    if !self.selected.insert(path.clone()) {
                        self.selected.remove(&path);
                    }
                }
            }
            Action::Yank => {
                let paths: Vec<PathBuf> = if self.selected.is_empty() {
                    if let Some(entry) = self.entries.get(self.cursor) {
                        vec![entry.path.clone()]
                    } else {
                        Vec::new()
                    }
                } else {
                    self.selected.iter().cloned().collect()
                };

                if !paths.is_empty() {
                    self.clipboard = Some((ClipboardOp::Copy, paths));
                    self.selected.clear(); // Clear selection after yank
                }
            }
            Action::Paste => {
                if let Some((op, entries)) = &self.clipboard {
                    match op {
                        ClipboardOp::Copy => {
                            for src in entries {
                                let file_name = src.file_name().unwrap_or_default();
                                let dest = self.cwd.join(file_name);
                                // Logic to avoid overwriting or handle collision?
                                // For now, simple copy.
                                let _ = ops::copy_recursive(src, &dest);
                            }
                        }
                    }
                    // Reload entries
                    if let Ok(entries) = read_entries(&self.cwd) {
                        self.entries = entries;
                    }
                }
            }
            Action::Delete => {
                let paths: Vec<PathBuf> = if self.selected.is_empty() {
                    if let Some(entry) = self.entries.get(self.cursor) {
                        vec![entry.path.clone()]
                    } else {
                        Vec::new()
                    }
                } else {
                    self.selected.iter().cloned().collect()
                };

                for path in paths {
                    let _ = ops::delete_path(&path);
                }
                self.selected.clear();
                if let Ok(entries) = read_entries(&self.cwd) {
                    self.entries = entries;
                    // Adjust cursor if out of bounds
                    if self.cursor >= self.entries.len() && !self.entries.is_empty() {
                        self.cursor = self.entries.len() - 1;
                    }
                }
            }
            Action::Chmod => {
                 if let Some(entry) = self.entries.get(self.cursor) {
                     if let Ok(meta) = std::fs::metadata(&entry.path) {
                         use std::os::unix::fs::PermissionsExt;
                         let mode = meta.permissions().mode();
                         self.popup = PopupState::Chmod {
                             path: entry.path.clone(),
                             mode,
                             cursor_idx: 0,
                         };
                     }
                 }
            }
            Action::Open => {
                if let Some(entry) = self.entries.get(self.cursor) {
                    // Use xdg-open on Linux
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&entry.path)
                        .spawn();
                }
            }
            Action::PreviewReady(content) => {
                self.preview = PreviewState::Ready(content);
            }
            Action::PreviewError { path, error } => {
                self.preview = PreviewState::Error {
                    _path: path,
                    message: error,
                };
            }
            Action::SwitchFocus => {
                self.active_focus = match self.active_focus {
                    ActiveFocus::FileList => ActiveFocus::Preview,
                    ActiveFocus::Preview => ActiveFocus::FileList,
                };
            }
            Action::ScrollPreviewUp => {
                if self.active_focus == ActiveFocus::Preview {
                    if self.preview_scroll > 0 {
                        self.preview_scroll -= 1;
                    }
                }
            }
            Action::ScrollPreviewDown => {
                if self.active_focus == ActiveFocus::Preview {
                    self.preview_scroll += 1;
                }
            }
            Action::ScrollPreviewPageUp => {
                if self.active_focus == ActiveFocus::Preview {
                    self.preview_scroll = self.preview_scroll.saturating_sub(10);
                }
            }
            Action::ScrollPreviewPageDown => {
                 if self.active_focus == ActiveFocus::Preview {
                    self.preview_scroll += 10;
                }
            }
            Action::PopupUp => {
                if let PopupState::Chmod { cursor_idx, .. } = &mut self.popup {
                    if *cursor_idx >= 3 {
                        *cursor_idx -= 3;
                    }
                }
            }
            Action::PopupDown => {
                if let PopupState::Chmod { cursor_idx, .. } = &mut self.popup {
                    if *cursor_idx < 6 {
                        *cursor_idx += 3;
                    }
                }
            }
            Action::PopupLeft => {
                if let PopupState::Chmod { cursor_idx, .. } = &mut self.popup {
                    if *cursor_idx % 3 > 0 {
                        *cursor_idx -= 1;
                    }
                }
            }
            Action::PopupRight => {
                if let PopupState::Chmod { cursor_idx, .. } = &mut self.popup {
                    if *cursor_idx % 3 < 2 {
                        *cursor_idx += 1;
                    }
                }
            }
            Action::PopupToggle => {
                if let PopupState::Chmod { mode, cursor_idx, .. } = &mut self.popup {
                    // Mapping idx 0-8 to mode bits
                    // Grid:
                    // Owner: R(0), W(1), X(2) -> 400, 200, 100
                    // Group: R(3), W(4), X(5) -> 040, 020, 010
                    // Other: R(6), W(7), X(8) -> 004, 002, 001
                    
                    let bit = match cursor_idx {
                        0 => 0o400, 1 => 0o200, 2 => 0o100,
                        3 => 0o040, 4 => 0o020, 5 => 0o010,
                        6 => 0o004, 7 => 0o002, 8 => 0o001,
                        _ => 0,
                    };
                    
                    if bit != 0 {
                        *mode ^= bit; // Toggle bit
                    }
                }
            }
            Action::PopupSubmit => {
                if let PopupState::Chmod { path, mode, .. } = &self.popup {
                     let _ = ops::set_permissions(path, *mode);
                     // Reload to update UI
                     if let Ok(entries) = read_entries(&self.cwd) {
                        self.entries = entries;
                     }
                }
                self.popup = PopupState::None;
            }
            Action::PopupCancel => {
                self.popup = PopupState::None;
            }
        }
    }
}

pub trait PreviewLoader {
    fn load(&self, path: PathBuf) -> Result<PreviewContent, String>;
}

pub struct DefaultPreviewLoader;

impl PreviewLoader for DefaultPreviewLoader {
    fn load(&self, path: PathBuf) -> Result<PreviewContent, String> {
        let title = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        if path.is_dir() {
            let mut tree = String::new();
            for entry in WalkDir::new(&path)
                .min_depth(1)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let depth = entry.depth();
                let indent = "  ".repeat(depth - 1);
                let name = entry.file_name().to_string_lossy();
                tree.push_str(&format!("{}|-- {}\n", indent, name));
            }
            return Ok(PreviewContent::Text {
                title,
                content: tree,
            });
        }

        // Try to load as image first
        if let Ok(reader) = image::ImageReader::open(&path) {
            if let Ok(dims) = reader.with_guessed_format() {
                if let Ok(img_dims) = dims.into_dimensions() {
                    return Ok(PreviewContent::Image {
                        title: title.clone(),
                        width: img_dims.0,
                        height: img_dims.1,
                        color_type: "Unknown".to_string(),
                    });
                }
            }
        }

        // Fallback: Check extension if image loading failed/wasn't supported format
        if let Some(ext) = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
        {
            match as_ref(ext.as_str()) {
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" => {
                    return Ok(PreviewContent::Image {
                        title,
                        width: 0,  // Unknown
                        height: 0, // Unknown
                        color_type: "Unknown (Metadata Load Failed)".to_string(),
                    });
                }
                _ => {}
            }
        }

        fn as_ref(s: &str) -> &str {
            s
        }

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                // Return raw content regardless of extension for now.
                // draw_preview handles highlighting.
                // TODO: For very large files, read only first N KB.
                Ok(PreviewContent::Text { title, content })
            }
            Err(_) => {
                let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;

                Ok(PreviewContent::Binary {
                    title,
                    size: meta.len(),
                })
            }
        }
    }
}

pub fn read_entries(path: &std::path::Path) -> std::io::Result<Vec<FsEntry>> {
    use std::os::unix::fs::PermissionsExt;

    let mut entries: Vec<FsEntry> = std::fs::read_dir(path)?
        .filter_map(|e| e.ok())
        .map(|entry| {
            let meta = entry.metadata().unwrap();
            let mode = meta.permissions().mode();
            
            // Format permissions logic
            let mut perms = String::with_capacity(10);
            perms.push(if meta.is_dir() { 'd' } else { '-' });
            perms.push(if mode & 0o400 != 0 { 'r' } else { '-' });
            perms.push(if mode & 0o200 != 0 { 'w' } else { '-' });
            perms.push(if mode & 0o100 != 0 { 'x' } else { '-' });
            perms.push(if mode & 0o040 != 0 { 'r' } else { '-' });
            perms.push(if mode & 0o020 != 0 { 'w' } else { '-' });
            let mut perms_str = String::with_capacity(10);
            perms_str.push(if entry.path().is_dir() { 'd' } else { '-' });
            perms_str.push(if mode & 0o400 != 0 { 'r' } else { '-' });
            perms_str.push(if mode & 0o200 != 0 { 'w' } else { '-' });
            perms_str.push(if mode & 0o100 != 0 { 'x' } else { '-' });
            perms_str.push(if mode & 0o040 != 0 { 'r' } else { '-' });
            perms_str.push(if mode & 0o020 != 0 { 'w' } else { '-' });
            perms_str.push(if mode & 0o010 != 0 { 'x' } else { '-' });
            perms_str.push(if mode & 0o004 != 0 { 'r' } else { '-' });
            perms_str.push(if mode & 0o002 != 0 { 'w' } else { '-' });
            perms_str.push(if mode & 0o001 != 0 { 'x' } else { '-' });

            FsEntry {
                path: entry.path().to_path_buf(),
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: entry.path().is_dir(),
                _size: entry.metadata().map(|m| m.len()).unwrap_or(0),
                permissions: perms_str,
            }
        })
        .collect();

    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir {
            b.is_dir.cmp(&a.is_dir) // Dirs first
        } else {
            a.name.cmp(&b.name) // Then alphabetical
        }
    });

    Ok(entries)
}

/* =========================
   RENDER (CLI DEMO)
========================= */

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect, Margin, Alignment},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Clear},
};

/* =========================
   TUI RENDER
========================= */

pub fn ui(f: &mut Frame, state: &mut AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    draw_file_list(f, state, chunks[0]);
    draw_preview(f, state, chunks[1]);

    // Draw Popup if active
    if let PopupState::Chmod { path, mode, cursor_idx } = &state.popup {
        let block = Block::default().title(" Permissions ").borders(Borders::ALL).style(Style::default().bg(Color::DarkGray));
        let size = f.size();
        let area = centered_rect(60, 20, size);
        f.render_widget(Clear, area); // Clear background
        f.render_widget(block, area);

        let inner = area.inner(&Margin { vertical: 1, horizontal: 1 });
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Title/Path
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Owner
                Constraint::Length(1), // Group
                Constraint::Length(1), // Other
                Constraint::Min(1),    // Spacer
                Constraint::Length(1), // Instructions
            ])
            .split(inner);

        let path_text = format!("Path: {}", path.file_name().unwrap_or_default().to_string_lossy());
        f.render_widget(Paragraph::new(path_text).alignment(Alignment::Center), chunks[0]);

        // Helper to draw row
        let draw_row = |label: &str, start_bit: u32, row_idx: usize| {
             let r_bit = start_bit;
             let w_bit = start_bit >> 1;
             let x_bit = start_bit >> 2;
             
             let r_check = if mode & r_bit != 0 { "[x]" } else { "[ ]" };
             let w_check = if mode & w_bit != 0 { "[x]" } else { "[ ]" };
             let x_check = if mode & x_bit != 0 { "[x]" } else { "[ ]" };
             
             // Check cursor
             let base_idx = row_idx * 3;
             let r_style = if *cursor_idx == base_idx { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
             let w_style = if *cursor_idx == base_idx + 1 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
             let x_style = if *cursor_idx == base_idx + 2 { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };

             let line = Line::from(vec![
                 Span::raw(format!("{:<10}", label)),
                 Span::styled(format!("R {}", r_check), r_style),
                 Span::raw("  "),
                 Span::styled(format!("W {}", w_check), w_style),
                 Span::raw("  "),
                 Span::styled(format!("X {}", x_check), x_style),
             ]);
             
             line
        };

        f.render_widget(Paragraph::new(draw_row("Owner", 0o400, 0)).alignment(Alignment::Center), chunks[2]);
        f.render_widget(Paragraph::new(draw_row("Group", 0o040, 1)).alignment(Alignment::Center), chunks[3]);
        f.render_widget(Paragraph::new(draw_row("Other", 0o004, 2)).alignment(Alignment::Center), chunks[4]);

        let help = "arrows: navigate | space: toggle | enter: save | esc: cancel";
        f.render_widget(Paragraph::new(help).style(Style::default().fg(Color::Gray)).alignment(Alignment::Center), chunks[6]);
    }
}

// Helper for centering popup
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_file_list(f: &mut Frame, state: &mut AppState, area: Rect) {
    let items: Vec<ListItem> = state
        .entries
        .iter()
        .map(|entry| {
            // Distinct icons
            let icon = if entry.is_dir { " " } else { " " };

            // Color logic:
            // Directories: Blue
            // Executables: Green (maybe later)
            // Symlinks: Cyan (maybe later)
            // Regular: White

            let color = if entry.is_dir {
                Color::Blue
            } else {
                Color::White
            };

            let style = if state.selected.contains(&entry.path) {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };

            // Layout: Name ... Permissions
            // Simple approach: Just append text. Ratatui list doesn't support columns easily without Table widget.
            // Let's pad it? Or just put it in parens?
            // "  FolderName (drwxr-xr-x)"

            ListItem::new(format!("{} {}  ({})", icon, entry.name, entry.permissions)).style(style)
        })
        .collect();

    let border_color = if state.active_focus == ActiveFocus::FileList {
        Color::Green
    } else {
        Color::White
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Files")
                .border_style(Style::default().fg(border_color)),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    let mut list_state = ListState::default();
    list_state.select(Some(state.cursor));

    f.render_stateful_widget(list, area, &mut list_state);
}

fn draw_preview(f: &mut Frame, state: &AppState, area: Rect) {
    let border_color = if state.active_focus == ActiveFocus::Preview {
        Color::Green
    } else {
        Color::White
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Preview")
        .border_style(Style::default().fg(border_color));

    match &state.preview {
        PreviewState::None => {
            f.render_widget(Paragraph::new("No preview").block(block), area);
        }
        PreviewState::Loading { .. } => {
            f.render_widget(Paragraph::new("Loading...").block(block), area);
        }
        PreviewState::Ready(content) => match content {
            PreviewContent::Text { title, content } => {
                let mut lines: Vec<Line> = Vec::new();

                let syntax = state
                    .syntax_set
                    .find_syntax_by_token(title)
                    .unwrap_or_else(|| state.syntax_set.find_syntax_plain_text());

                let mut h =
                    HighlightLines::new(syntax, &state.theme_set.themes["base16-ocean.dark"]);

                // PERFORMANCE FIX: Only highlight visible lines
                // Skip lines based on scroll
                let scroll = state.preview_scroll;
                let height = area.height as usize;

                // We use LinesWithEndings to ensure correct highlighting context if we were keeping state,
                // but since we create new HighlightLines each frame, we assume stateless highlighting (ok for most langs).
                // Actually syntect is stateful. Ideally we should iterate from start but that's slow.
                // For now, re-instantiating is the compromise for performance vs correctness.
                // But `highlight_line` updates state. We need to feed it previous lines?
                // For large files, that's slow.
                // Let's just highlight the slice. It might be slightly wrong for multi-line constructs but fast.

                for line in content.lines().skip(scroll).take(height) {
                    // Sanitize line: Remove control chars (like \r) but keep tabs/spaces.
                    // This prevents cursor jumping or terminal corruption.
                    let clean_line: String = line
                        .chars()
                        .filter(|c| !c.is_control() || *c == '\t')
                        .collect();

                    let ranges: Vec<(SyntectStyle, &str)> = h
                        .highlight_line(&clean_line, &state.syntax_set)
                        .unwrap_or_default();
                    let spans: Vec<Span> = ranges
                        .into_iter()
                        .map(|(style, text)| {
                            Span::styled(
                                text.to_string(),
                                Style::default().fg(Color::Rgb(
                                    style.foreground.r,
                                    style.foreground.g,
                                    style.foreground.b,
                                )),
                            )
                        })
                        .collect();
                    lines.push(Line::from(spans));
                }

                let p = Paragraph::new(lines).block(block.title(title.as_str()));
                // .scroll() removed because we manually sliced content
                f.render_widget(p, area);
            }
            PreviewContent::Binary { title, size } => {
                let text = format!("Binary file\nSize: {} bytes", size);
                let p = Paragraph::new(text).block(block.title(title.as_str()));
                f.render_widget(p, area);
            }
            PreviewContent::Image {
                title,
                width,
                height,
                color_type,
            } => {
                let dim_text = if *width == 0 && *height == 0 {
                    "Dimensions: Unavailable".to_string()
                } else {
                    format!("Dimensions: {} x {} px", width, height)
                };

                let text = vec![
                    Line::from(vec![Span::styled(
                        "Image File",
                        Style::default().add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(dim_text),
                    Line::from(format!("Info: {}", color_type)),
                    Line::from(""),
                    Line::from(vec![Span::styled(
                        "Press 'o' to open externally.",
                        Style::default().fg(Color::DarkGray),
                    )]),
                ];
                let p = Paragraph::new(text).block(block.title(title.as_str()));
                f.render_widget(p, area);
            }
        },
        PreviewState::Error { message, .. } => {
            let p = Paragraph::new(format!("Error: {}", message))
                .block(block.title("Error"))
                .style(Style::default().fg(Color::Red));
            f.render_widget(p, area);
        }
    }
}
