use std::collections::BTreeSet;
use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tokn_session_core::{AgentEvent, LoadedSession};

use crate::render::{event_type, render_event_pretty, render_event_summary};

pub fn browse_session(session: &LoadedSession) -> Result<(), String> {
  let mut terminal = BrowserTerminal::enter()?;
  let result = EventBrowser::new(session).run(&mut terminal.terminal);
  terminal.leave()?;
  result
}

struct BrowserTerminal {
  terminal: Terminal<CrosstermBackend<Stdout>>,
  restored: bool,
}

impl BrowserTerminal {
  fn enter() -> Result<Self, String> {
    enable_raw_mode().map_err(|err| format!("failed to enable raw mode: {err}"))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|err| format!("failed to enter alternate screen: {err}"))?;
    let terminal =
      Terminal::new(CrosstermBackend::new(stdout)).map_err(|err| format!("failed to create terminal: {err}"))?;
    Ok(Self {
      terminal,
      restored: false,
    })
  }

  fn leave(&mut self) -> Result<(), String> {
    if self.restored {
      return Ok(());
    }
    disable_raw_mode().map_err(|err| format!("failed to disable raw mode: {err}"))?;
    execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
      .map_err(|err| format!("failed to leave alternate screen: {err}"))?;
    self.restored = true;
    Ok(())
  }
}

impl Drop for BrowserTerminal {
  fn drop(&mut self) {
    let _ = self.leave();
  }
}

struct EventBrowser {
  session_id: String,
  rows: Vec<EventRow>,
  selected: usize,
  scroll: usize,
  expanded: BTreeSet<usize>,
}

impl EventBrowser {
  fn new(session: &LoadedSession) -> Self {
    Self {
      session_id: session.reference.id.clone(),
      rows: session.events.iter().enumerate().map(EventRow::new).collect(),
      selected: 0,
      scroll: 0,
      expanded: BTreeSet::new(),
    }
  }

  fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), String> {
    loop {
      terminal
        .draw(|frame| self.draw(frame))
        .map_err(|err| format!("failed to draw browser: {err}"))?;

      if !event::poll(Duration::from_millis(200)).map_err(|err| format!("failed to poll events: {err}"))? {
        continue;
      }

      if let Event::Key(key) = event::read().map_err(|err| format!("failed to read event: {err}"))? {
        if self.handle_key(key) {
          return Ok(());
        }
      }
    }
  }

  fn draw(&mut self, frame: &mut ratatui::Frame<'_>) {
    let area = frame.area();
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(area);
    let help = format!(
      "Session {}  {}/{}  j/k move  h/l collapse/expand  Enter toggle  z solo  C collapse all  q quit",
      self.session_id,
      self.selected.saturating_add(1),
      self.rows.len()
    );
    frame.render_widget(
      Paragraph::new(help).block(Block::default().borders(Borders::BOTTOM)),
      chunks[0],
    );

    let height = chunks[1].height as usize;
    self.keep_selected_visible(height);
    let lines = self.visible_lines(height);
    frame.render_widget(Paragraph::new(lines), chunks[1]);
  }

  fn handle_key(&mut self, key: KeyEvent) -> bool {
    match key.code {
      KeyCode::Char('q') | KeyCode::Esc => return true,
      KeyCode::Char('j') | KeyCode::Down => self.select_next(),
      KeyCode::Char('k') | KeyCode::Up => self.select_previous(),
      KeyCode::Char('g') | KeyCode::Home => self.select_first(),
      KeyCode::Char('G') | KeyCode::End => self.select_last(),
      KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => self.page_down(),
      KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => self.page_up(),
      KeyCode::Char('h') => self.collapse_selected(),
      KeyCode::Char('l') => self.expand_selected(),
      KeyCode::Char('z') => self.solo_selected(),
      KeyCode::Char('C') => self.expanded.clear(),
      KeyCode::Enter | KeyCode::Char(' ') => self.toggle_selected(),
      _ => {}
    }
    false
  }

  fn select_next(&mut self) {
    if self.selected + 1 < self.rows.len() {
      self.selected += 1;
    }
  }

  fn select_previous(&mut self) {
    self.selected = self.selected.saturating_sub(1);
  }

  fn select_first(&mut self) {
    self.selected = 0;
  }

  fn select_last(&mut self) {
    if !self.rows.is_empty() {
      self.selected = self.rows.len() - 1;
    }
  }

  fn page_down(&mut self) {
    self.selected = (self.selected + 10).min(self.rows.len().saturating_sub(1));
  }

  fn page_up(&mut self) {
    self.selected = self.selected.saturating_sub(10);
  }

  fn collapse_selected(&mut self) {
    self.expanded.remove(&self.selected);
  }

  fn expand_selected(&mut self) {
    self.expanded.insert(self.selected);
  }

  fn toggle_selected(&mut self) {
    if !self.expanded.remove(&self.selected) {
      self.expanded.insert(self.selected);
    }
  }

  fn solo_selected(&mut self) {
    self.expanded.clear();
    self.expanded.insert(self.selected);
  }

  fn keep_selected_visible(&mut self, height: usize) {
    if height == 0 {
      return;
    }
    if self.selected < self.scroll {
      self.scroll = self.selected;
    }
    if self.selected >= self.scroll + height {
      self.scroll = self.selected.saturating_sub(height - 1);
    }
  }

  fn visible_lines(&self, height: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for index in self.scroll..self.rows.len() {
      if lines.len() >= height {
        break;
      }
      let row = &self.rows[index];
      let selected = index == self.selected;
      let expanded = self.expanded.contains(&index);
      lines.push(row.header_line(index, selected, expanded));
      if expanded {
        for detail in row.detail_lines() {
          if lines.len() >= height {
            break;
          }
          lines.push(detail);
        }
      }
    }
    lines
  }
}

struct EventRow {
  kind: &'static str,
  summary: String,
  detail: String,
}

impl EventRow {
  fn new((index, event): (usize, &AgentEvent)) -> Self {
    let _ = index;
    Self {
      kind: event_type(event),
      summary: render_event_summary(event),
      detail: render_event_pretty(event),
    }
  }

  fn header_line(&self, index: usize, selected: bool, expanded: bool) -> Line<'static> {
    let cursor = if selected { ">" } else { " " };
    let fold = if expanded { "-" } else { "+" };
    let text = format!("{cursor} {:04} {fold} {:<9} {}", index + 1, self.kind, self.summary);
    if selected {
      Line::from(Span::styled(text, Style::default().add_modifier(Modifier::REVERSED)))
    } else {
      Line::from(text)
    }
  }

  fn detail_lines(&self) -> Vec<Line<'static>> {
    self
      .detail
      .trim_matches('\n')
      .lines()
      .map(|line| Line::from(format!("      {line}")))
      .collect()
  }
}
