use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Widget},
    Frame,
};
use std::collections::VecDeque;

const HISTORY_LEN: usize = 60;

pub struct AppState {
    pub audio_levels: VecDeque<f32>,
    pub transcript: String,
    pub status: String,
    pub show_device_picker: bool,
    pub device_items: Vec<String>,
    pub device_ids: Vec<u32>,
    pub picker_idx: usize,
    pub volume: f32,
    pub muted: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            audio_levels: VecDeque::with_capacity(HISTORY_LEN),
            transcript: String::new(),
            status: "idle".into(),
            show_device_picker: false,
            device_items: Vec::new(),
            device_ids: Vec::new(),
            picker_idx: 0,
            volume: 1.0,
            muted: false,
        }
    }

    pub fn push_level(&mut self, level: f32) {
        if self.audio_levels.len() >= HISTORY_LEN {
            self.audio_levels.pop_front();
        }
        self.audio_levels.push_back(level.clamp(0.0, 1.0));
    }

    pub fn update_transcript(&mut self, text: impl Into<String>) {
        self.transcript = text.into();
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    pub fn adjust_volume(&mut self, delta: f32) {
        self.volume = (self.volume + delta).clamp(0.0, 1.0);
    }
}



struct InputWidget {
    device_name: String,
    volume: f32,
    muted: bool,
    current_level: f32,
}

impl Widget for InputWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = "Input";
        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height < 6 || inner.width == 0 {
            return;
        }

        // Device name plain label
        let name_y = inner.y + 1;
        // Clear line to avoid leftover characters
        for x in inner.x..inner.x + inner.width as u16 {
            buf[(x, name_y)].set_symbol(" ");
        }
        let name_line = self.device_name;
        for (i, ch) in name_line.chars().enumerate() {
            let x = inner.x + 1 + i as u16;
            let y = name_y;
            if x < inner.x + inner.width as u16 {
                buf[(x, y)].set_symbol(&ch.to_string());
            }
        }

        // Horizontal volume bar below device label
        let vol_y = inner.y + 3;
        for (k, ch) in "Volume".chars().enumerate() {
            buf[(inner.x + 1 + k as u16, vol_y)].set_symbol(&ch.to_string());
        }
        let bar_start = inner.x + 8;
        let bar_w = inner.width.saturating_sub(9) as usize;
        let effective_vol = if self.muted { 0.0 } else { self.volume };
        let filled = (bar_w as f32 * effective_vol).round() as usize;
        for col in 0..bar_w {
            let x = bar_start + col as u16;
            let sym = if col < filled { "█" } else { "░" };
            let colr = if col < filled { Color::Magenta } else { Color::DarkGray };
            buf[(x, vol_y)].set_symbol(sym).set_fg(colr);
        }
        let pct = if self.muted {
            "MUTED".to_string()
        } else {
            format!("{:3.0}%", self.volume * 100.0)
        };
        // Center volume value inside the bar
        let text_start = bar_start + (bar_w as u16).saturating_sub(pct.len() as u16) / 2;
        for (i, ch) in pct.chars().enumerate() {
            let x = text_start + i as u16;
            if x < bar_start + bar_w as u16 && x < inner.x + inner.width {
                buf[(x, vol_y)].set_symbol(&ch.to_string()).set_fg(if self.muted { Color::Red } else { Color::White });
            }
        }

        // Level label and horizontal dB meter on same line
        let level_y = inner.y + 5;
        for (i, ch) in "Level".chars().enumerate() {
            buf[(inner.x + 1 + i as u16, level_y)].set_symbol(&ch.to_string());
        }
        let db = if self.current_level > 1e-6 { 20.0 * self.current_level.log10() } else { -100.0 };
        let db_clamped = db.clamp(-60.0, 0.0);
        let norm = ((db_clamped + 60.0) / 60.0).clamp(0.0, 1.0);
        let meter_w = inner.width.saturating_sub(9) as usize;
        let filled_db = (meter_w as f32 * norm).round() as usize;
        let color = if self.current_level > 0.8 { Color::Red } else if self.current_level > 0.5 { Color::Yellow } else if self.current_level > 0.2 { Color::Green } else { Color::DarkGray };
        for col in 0..meter_w {
            let x = bar_start + col as u16;
            let sym = if col < filled_db { "█" } else { "░" };
            buf[(x, level_y)].set_symbol(sym).set_fg(color);
        }

        // Four dB markings under meter
        let marks_y = inner.y + 6;
        let marks = [-60.0, -40.0, -20.0, 0.0];
        for &m in &marks {
            let pos = ((m + 60.0) / 60.0 * (meter_w - 2) as f32).round() as usize;
            let label = format!("{:.0}", m);
            for (i, ch) in label.chars().enumerate() {
                let xx = bar_start + 1 + pos as u16 - label.len() as u16 / 2 + i as u16;
                if xx < inner.x + inner.width as u16 {
                    buf[(xx, marks_y)].set_symbol(&ch.to_string()).set_fg(Color::Gray);
                }
            }
        }
    }
}

pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(Line::from(vec![
        Span::styled("scrivener", Style::default().add_modifier(Modifier::BOLD).fg(Color::White)),
        Span::raw("  "),
        Span::styled(format!("status: {}", state.status), Style::default().fg(Color::Gray)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[1]);

    let current_level = *state.audio_levels.back().unwrap_or(&0.0);
    let device_name = state.device_items.get(state.picker_idx).cloned().unwrap_or_else(|| "No device".to_string());
    let input_widget = InputWidget { device_name, volume: state.volume, muted: state.muted, current_level };
    frame.render_widget(input_widget, main[0]);

    let transcript = Paragraph::new(state.transcript.as_str())
        .block(Block::default().title("Transcription").borders(Borders::ALL).border_style(Style::default().fg(Color::Magenta)))
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(transcript, main[1]);

    let footer = Paragraph::new("d device  +/- volume  q quit")
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);

    if state.show_device_picker {
        let max_w = 50u16;
        let max_h = 10u16;
        let width = max_w.min(frame.area().width);
        let height = max_h.min(frame.area().height);
        let x = frame.area().x + (frame.area().width.saturating_sub(width)) / 2;
        let y = frame.area().y + (frame.area().height.saturating_sub(height)) / 2;
        let area = Rect { x, y, width, height };

        let items: Vec<ListItem> = state.device_items.iter().map(|s| ListItem::new(s.clone())).collect();

        let mut list_state = ListState::default();
        list_state.select(Some(state.picker_idx));

        frame.render_widget(ratatui::widgets::Clear, area);

        let list = List::new(items)
            .block(
                Block::default()
                    .title("Select Input Device")
                    .borders(Borders::ALL)
                    .padding(ratatui::widgets::Padding::uniform(1))
            )
            .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

        frame.render_stateful_widget(list, area, &mut list_state);
    }
}
