use crossterm::{
    event::{
        self, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame, Terminal,
};
use regex::RegexSet;
use std::{
    collections::VecDeque,
    io::{self, Stdout},
    time::{Duration, Instant},
};
use std::io::ErrorKind;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

lazy_static::lazy_static! {
    static ref REGSET: RegexSet = RegexSet::new([
        r"^(\x60|\.|:|/|-|\+|o|s|h|d|y| ){50,}",      // ASCII Chicken
        r"^# ",                                       // # command
        r"(?m)^\s*(-|=|#)+\s*$",                      // ================
        r"^\[ =+ ?.* ?=+ \]",                         // [ ===== Headline ====== ]
        r"^> \w+",                                    // > Finished job
        r"^(ERROR)|(WARNING): ",                      // ERROR: something went wrong :(
        r"^.*: +.*",                                  // -arg: value
        r"^\[.*\]",                                   // [default=something]
        r"(?m)^\S+( \[?-\S*( <\S*>)?\]?)*\s*$",       // command [-arg <value>] [-flag]
    ]).unwrap();

    static ref COLORSET: [(Color, Modifier);9] = [
        (Color::White, Modifier::empty()),  // # command
        (Color::White, Modifier::BOLD),   // # command
        (Color::Blue, Modifier::empty()),   // ================
        (Color::Yellow, Modifier::BOLD),  // [ ===== Headline ====== ]
        (Color::Cyan, Modifier::empty()),   // > Finished job
        (Color::Red, Modifier::empty()),    // ERROR: something went wrong :(
        (Color::Green, Modifier::empty()),  // -arg value
        (Color::Green, Modifier::BOLD),   // [default=something]
        (Color::Yellow, Modifier::empty()), // command [-arg <value>] [-flag]
    ];
}

struct InterruptHandler(VecDeque<Instant>);

impl InterruptHandler {
    fn new(cap: usize) -> Self {
        Self(VecDeque::with_capacity(cap))
    }
    fn interrupted(&mut self) -> bool {
        if self.0.len() == 3 {
            if let Some(time) = self.0.pop_back() {
                if Instant::now() - time <= Duration::new(3, 0) {
                    true
                } else {
                    self.0.push_front(Instant::now());
                    false
                }
            } else { false }
        } else {
            self.0.push_front(Instant::now());
            false
        }
    }
}

struct History {
    hist: Vec<String>,
    index: usize,
}

impl History {
    fn new() -> Self {
        Self {
            hist: vec!["".to_string()],
            index: 0,
        }
    }
    fn prev_cmd(&mut self) -> String {
        if self.index > 0 {
            self.index -= 1;
        }
        self.hist[self.index].to_string()
    }
    fn next_cmd(&mut self) -> String {
        if self.index < self.hist.len() - 1 {
            self.index += 1;
        }
        self.hist[self.index].to_string()
    }
    fn add(&mut self, entry: String) {
        self.hist.insert(self.hist.len() - 1, entry)
    }
    fn reset(&mut self) {
        self.index = self.hist.len() - 1
    }
}

enum InputMode {
    Normal,
    Insert,
}

/// App holds the state of the application
pub struct App {
    /// Current value of the input box
    input: String,
    /// All application output
    output: Vec<String>,
    /// History of commands entered
    cmd_history: History,
    /// User-controlled scrolling
    manual_scroll: bool,
    /// Scrollbar State
    scrollbar: ScrollbarState,
    /// Scroll position
    scroll_pos: usize,
    /// Cursor Position
    cursor_pos: usize,
    /// Input Mode
    input_mode: InputMode,
}

impl<'a> App {
    pub fn new() -> Self {
        Self {
            input: String::default(),
            output: Vec::new(),
            cmd_history: History::new(),
            manual_scroll: false,
            scrollbar: ScrollbarState::default(),
            scroll_pos: 0,
            cursor_pos: 0,
            input_mode: InputMode::Insert,
        }
    }

    fn delete_char(&mut self) {
        if self.cursor_pos != 0 {
            self.remove_char(self.cursor_pos)
        }
    }

    fn submit(&mut self) -> String {
        let entr_txt: String = self.input.drain(..).collect();

        self.output.push(entr_txt.clone());
        self.cmd_history.add(entr_txt.clone());
        self.cmd_history.reset();
        self.cursor_reset();

        entr_txt
    }

    fn put_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_right();
    }

    fn cursor_left(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_sub(1).clamp(0, self.input.len());
    }

    fn cursor_right(&mut self) {
        self.cursor_pos = self.cursor_pos.saturating_add(1).clamp(0, self.input.len());
    }

    fn cursor_reset(&mut self) {
        self.cursor_pos = 0
    }

    fn scroll_up(&mut self) {
        self.scroll_pos = self.scroll_pos.saturating_sub(1);
        self.scrollbar = self.scrollbar.position(self.scroll_pos);
        self.manual_scroll = true;
    }

    fn scroll_down(&mut self) {
        self.scroll_pos = self.scroll_pos.saturating_add(1);
        self.scrollbar = self.scrollbar.position(self.scroll_pos);
    }

    fn remove_char(&mut self, idx: usize) {
        let left_idx = self.cursor_pos - 1;

        let left_part = self.input.chars().take(left_idx);
        let right_part = self.input.chars().skip(idx);

        self.input = left_part.chain(right_part).collect();
        self.cursor_left();
    }

    fn parse<S: AsRef<str>>(s: S) -> Line<'a> {
        let matches: Vec<_> = REGSET.matches(s.as_ref()).into_iter().collect();

        let (color, modf) = if !matches.is_empty() {
            COLORSET[matches[0]]
        } else {
            (Color::White, Modifier::empty())
        };
        Line::styled(
            s.as_ref().to_string(),
            Style::default().fg(color).add_modifier(modf),
        )
    }

    /// Start render loop
    pub async fn run(
        mut self,
        input_tx: UnboundedSender<String>,
        mut output_rx: UnboundedReceiver<String>,
        tick_rate: Duration,
    ) -> io::Result<()> {
        let mut spam_handler = InterruptHandler::new(3);
        let stdout = io::stdout();
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;
        let mut prev_tick = Instant::now();
        let mut res: io::Result<()> = Ok(());

        // setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        loop {
            terminal.draw(|f| self.ui(f))?;

            if let Ok(str) = output_rx.try_recv() {
                self.output.push(str)
            }

            let timeout = tick_rate.saturating_sub(prev_tick.elapsed());
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match self.input_mode {
                            InputMode::Insert => {
                                match key.code {
                                    KeyCode::Enter => {
                                        let entr_txt: String = self.submit();
                                        input_tx.send(format!("{}\r\n", entr_txt.clone())).unwrap();
                                        if entr_txt.to_uppercase() == "EXIT" {
                                            break;
                                        }
                                    }
                                    KeyCode::Char('c')
                                    if key.modifiers == KeyModifiers::from_name("CONTROL").unwrap() =>
                                        {
                                            if input_tx.send("stop\n".to_string()).is_err() {
                                                self.output.push("Couldn't stop!".to_string());
                                            }
                                            if spam_handler.interrupted() {
                                                res = input_tx.send("EXIT".to_string()).map_err(|e| io::Error::new(ErrorKind::Other, e.0));
                                                break;
                                            }
                                        }
                                    KeyCode::Char(c) => self.put_char(c),
                                    KeyCode::Backspace => self.delete_char(),
                                    KeyCode::Up => {
                                        self.input = self.cmd_history.prev_cmd();
                                        self.cursor_pos = self.input.len();
                                    }
                                    KeyCode::Down => {
                                        self.input = self.cmd_history.next_cmd();
                                        self.cursor_pos = self.input.len();
                                    }
                                    KeyCode::Left => self.cursor_left(),
                                    KeyCode::Right => self.cursor_right(),
                                    KeyCode::PageUp => self.scroll_up(),
                                    KeyCode::PageDown => self.scroll_down(),
                                    KeyCode::Esc => self.input_mode = InputMode::Normal,

                                    _ => (),
                                }
                            }
                            InputMode::Normal => {
                                match key.code {
                                    KeyCode::Up | KeyCode::PageUp => self.scroll_up(),
                                    KeyCode::Down | KeyCode::PageDown => self.scroll_down(),
                                    KeyCode::Esc => self.input_mode = InputMode::Insert,
                                    _ => ()
                                }
                            }
                        }
                    }
                }
            }

            if prev_tick.elapsed() >= tick_rate {
                prev_tick = Instant::now();
            }
        }
        Self::shutdown(terminal)?;

        res
    }

    fn ui(&mut self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
            .split(f.size());

        let (msg_color, input_color) = match self.input_mode {
            InputMode::Insert => (Color::Yellow, Color::White),
            InputMode::Normal => (Color::White, Color::Yellow)
        };

        // Set scroll position
        let lines: Vec<Line> = self.output.iter().map(Self::parse).collect();
        let box_height = chunks[0].height as usize;
        let visible_len = (lines.len() as isize - box_height as isize + 2).clamp(0, lines.len() as isize);
        if !self.manual_scroll {
            self.scroll_pos = visible_len as usize;
        } else if self.scroll_pos >= visible_len as usize {
            self.manual_scroll = false;
        }
        self.scrollbar = self.scrollbar.content_length(lines.len());

        // Message Box
        let messages = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(msg_color)).title("Messages"))
            .scroll((self.scroll_pos as u16, 0));
        f.render_widget(messages, chunks[0]);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("^"))
                .end_symbol(Some("v")),
            chunks[0],
            &mut self.scrollbar,
        );

        // Input Box
        let input = Paragraph::new(self.input.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(input_color)).title("Input"));
        f.render_widget(input, chunks[1]);
        // Show cursor
        f.set_cursor(
            // Put cursor after input text
            chunks[1].x + self.cursor_pos as u16 + 1,
            // Leave room for border
            chunks[1].y + 1,
        );
    }

    /// restore terminal
    fn shutdown(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen
        )?;
        terminal.show_cursor()?;
        Ok(())
    }
}
