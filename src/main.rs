mod app;
mod ops;

use std::{collections::HashSet, io, time::Duration};

use app::{
    Action, ActiveFocus, AppState, DefaultPreviewLoader, PreviewLoader, PreviewState, Reducer,
    read_entries, ui,
};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use syntect::{highlighting::ThemeSet, parsing::SyntaxSet};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Setup Terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create App State
    let cwd = std::env::current_dir()?;
    let entries = read_entries(&cwd)?; // Used from app module

    let syntax_set = SyntaxSet::load_defaults_newlines();
    let theme_set = ThemeSet::load_defaults();

    let mut state = AppState {
        cwd,
        entries,
        cursor: 0,
        selected: HashSet::new(),
        preview: PreviewState::None,
        syntax_set,
        theme_set,
        clipboard: None,
        active_focus: ActiveFocus::FileList,
        preview_scroll: 0,
        popup: app::PopupState::None,
    };

    let loader = DefaultPreviewLoader;
    let res = run_app(&mut terminal, &mut state, &loader);

    // Restore Terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    state: &mut AppState,
    loader: &impl PreviewLoader,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, state))?;

        if crossterm::event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    // Check for Popup State first
                    match state.popup {
                        app::PopupState::None => {
                            match key.code {
                                KeyCode::Char('q') => return Ok(()),

                                // Focus Switching
                                KeyCode::Tab | KeyCode::Char('h')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    state.reduce(Action::SwitchFocus);
                                }

                                // Navigation / Scrolling (Context Aware)
                                KeyCode::Char('j') | KeyCode::Down => {
                                    if state.active_focus == ActiveFocus::Preview {
                                        state.reduce(Action::ScrollPreviewDown);
                                    } else {
                                        state.reduce(Action::CursorMoveDown);
                                    }
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    if state.active_focus == ActiveFocus::Preview {
                                        state.reduce(Action::ScrollPreviewUp);
                                    } else {
                                        state.reduce(Action::CursorMoveUp);
                                    }
                                }

                                // Page Scrolling
                                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    state.reduce(Action::ScrollPreviewPageUp);
                                }
                                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    state.reduce(Action::ScrollPreviewPageDown);
                                }
                                // Page Up/Down keys
                                KeyCode::PageUp => state.reduce(Action::ScrollPreviewPageUp),
                                KeyCode::PageDown => state.reduce(Action::ScrollPreviewPageDown),

                                KeyCode::Char(' ') => state.reduce(Action::ToggleSelect),
                                KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                                    if state.active_focus == ActiveFocus::FileList {
                                        state.reduce(Action::EnterDir);
                                    }
                                }
                                KeyCode::Backspace | KeyCode::Char('h') | KeyCode::Left
                                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    if state.active_focus == ActiveFocus::FileList {
                                        state.reduce(Action::GoBack);
                                    }
                                }
                                KeyCode::Char('y') => state.reduce(Action::Yank),
                                KeyCode::Char('P') => state.reduce(Action::Paste),
                                KeyCode::Char('d') => state.reduce(Action::Delete),
                                KeyCode::Char('x') => state.reduce(Action::Chmod),
                                KeyCode::Char('o') => state.reduce(Action::Open),
                                KeyCode::Char('p') => {
                                    if let Some(entry) = state.entries.get(state.cursor) {
                                        let path = entry.path.clone();
                                        state.reduce(Action::RequestPreview(path.clone()));

                                        match loader.load(path.clone()) {
                                            Ok(content) => {
                                                state.reduce(Action::PreviewReady(content));
                                            }
                                            Err(e) => {
                                                // Actually logic uses path in PreviewError variant, but we renamed field in enum definition to _path?
                                                // Wait, I renamed field in `PreviewState::Error { _path, message }`.
                                                // But `Action::PreviewError` is a separate enum variant!
                                                // Let's check `Action` definition in `app.rs`.
                                                state.reduce(Action::PreviewError { path, error: e });
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {
                           // ... popup keys handling (kept same)
                           // Wait, I need to replicate the popup block or else it's outside this match
                           // Actually the user loop provided in replacement covers the 'None' arm.
                           // I should include the `_` arm in this replacement to be safe and clean.
                           
                            // Popup is active, handle popup keys
                            match key.code {
                                KeyCode::Esc | KeyCode::Char('q') => state.reduce(Action::PopupCancel),
                                KeyCode::Enter => state.reduce(Action::PopupSubmit),
                                KeyCode::Up | KeyCode::Char('k') => state.reduce(Action::PopupUp),
                                KeyCode::Down | KeyCode::Char('j') => state.reduce(Action::PopupDown),
                                KeyCode::Left | KeyCode::Char('h') => state.reduce(Action::PopupLeft),
                                KeyCode::Right | KeyCode::Char('l') => state.reduce(Action::PopupRight),
                                KeyCode::Char(' ') | KeyCode::Char('x') => state.reduce(Action::PopupToggle),
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }
}
