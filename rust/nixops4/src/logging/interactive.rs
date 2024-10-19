use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, KeyCode},
    terminal::{self, ClearType},
    ExecutableCommand,
};
use nix::unistd::{dup, dup2, pipe};
use ratatui::{
    layout::Rect,
    prelude::CrosstermBackend,
    style::{Color, Modifier},
    widgets::{Paragraph, Wrap},
    Frame, Terminal, Viewport,
};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{self, BufRead as _, Write},
    os::fd::{AsRawFd as _, FromRawFd},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};
use tracing_subscriber::{
    layer::SubscriberExt as _,
    registry::{LookupSpan as _, SpanData},
};

use crate::interrupt::InterruptState;

use super::Frontend;

pub(crate) struct InteractiveLogger {
    interrupt_state: InterruptState,
    headless_logger: super::headless::HeadlessLogger,
    log_shovel_thread: Option<thread::JoinHandle<()>>,
    tui_thread: Option<thread::JoinHandle<Result<()>>>,
    orig_stderr: Option<File>,
    orig_stdout: Option<File>,
    active_spans: Arc<Mutex<BTreeSet<u64>>>,
}
impl InteractiveLogger {
    pub(crate) fn new(interrupt_state: InterruptState) -> Self {
        Self {
            interrupt_state,
            headless_logger: super::headless::HeadlessLogger {},
            log_shovel_thread: None,
            tui_thread: None,
            orig_stderr: None,
            orig_stdout: None,
            active_spans: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }
}
impl Drop for InteractiveLogger {
    fn drop(&mut self) {
        self.tear_down().map_or_else(
            |e| eprintln!("error while tearing down interactive logger: {:?}", e),
            |_| {},
        );
    }
}

impl Frontend for InteractiveLogger {
    fn set_up(&mut self, options: &super::Options) -> Result<()> {
        // Shuffle file descriptors around to capture all logs
        self.orig_stderr = Some(unsafe {
            let stderr2 = dup(2).context("dup stderr")?;
            std::fs::File::from_raw_fd(stderr2)
        });
        self.orig_stdout = Some(unsafe {
            let stdout2 = dup(1).context("dup stdout")?;
            std::fs::File::from_raw_fd(stdout2)
        });

        // Use an internal pipe for logging
        let (diag_read, diag_write) = pipe().context("pipe")?;
        dup2(diag_write.as_raw_fd(), 1).context("dup2 stdout")?;
        dup2(diag_write.as_raw_fd(), 2).context("dup2 stderr")?;

        let (diag_queue_sender, diag_queue_receiver) = mpsc::channel::<String>();

        let log_shovel_thread = thread::spawn(move || {
            let diag_read = std::fs::File::from(diag_read);
            let mut diag_read = io::BufReader::new(diag_read);
            let mut buf = String::new();
            loop {
                let r = diag_read.read_line(&mut buf);
                if buf.ends_with('\n') {
                    buf.pop();
                    if buf.ends_with('\r') {
                        buf.pop();
                    }
                }
                match r {
                    Ok(0) => break,
                    Ok(_) => {
                        diag_queue_sender.send(std::mem::take(&mut buf)).unwrap();
                    }
                    Err(e) => {
                        panic!("error reading from diagnostics pipe: {:?}", e);
                    }
                }
            }
        });
        self.log_shovel_thread = Some(log_shovel_thread);

        let interrupt_state = self.interrupt_state.clone();

        let logger = Arc::new(self.headless_logger.make_subscriber(options)?);
        // We use the logger as a reference to the registry, containing span data (except active spans)
        let registry_ref = logger.clone();
        let logger = logger.with(SpanCollector::new(self.active_spans.clone()));
        let active_spans = self.active_spans.clone();

        let tui_thread = spawn_log_ui(
            self.interrupt_state.clone(),
            self.orig_stderr
                .as_mut()
                .unwrap()
                .try_clone()
                .expect("clone stderr"),
            diag_queue_receiver,
            Arc::new(Box::new(move |frame: &mut Frame| {
                let tui_area = frame.area();
                let time = std::time::SystemTime::now();

                let spinner = (time
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
                    / 125) as usize;

                let text = format!(
                    "{}{}{}",
                    "▄▀      ".chars().nth(spinner % 8).unwrap(),
                    "  ▀▄  ▀▄".chars().nth(spinner % 8).unwrap(),
                    "    ▄▀  ".chars().nth(spinner % 8).unwrap(),
                );

                let spinner_paragraph = Paragraph::new(text)
                    .style(
                        ratatui::style::Style::default()
                            .fg(Color::Reset)
                            .bg(Color::Reset)
                            .add_modifier(Modifier::BOLD),
                    )
                    .alignment(ratatui::layout::Alignment::Center);

                let border_color = if interrupt_state.is_interrupted() {
                    ratatui::style::Color::Yellow
                } else {
                    ratatui::style::Color::Blue
                };
                let title = if interrupt_state.is_interrupted() {
                    "Stopping"
                } else {
                    "Running"
                };

                let spans_paragraph = {
                    let x = active_spans.as_ref().lock().expect("active_spans lock");
                    let text = x
                        .iter()
                        .map(|id| {
                            let id = tracing::Id::from_u64(*id);
                            let data = registry_ref.span_data(&id);
                            match data {
                                Some(data) => format!("{}", data.metadata().name()),
                                None => format!("<unknown {:?}>", id),
                            }
                        })
                        .collect::<Vec<String>>()
                        .join("\n");

                    Paragraph::new(format!("Current activities:\n{}", text))
                        .style(ratatui::style::Style::default().fg(Color::Reset))
                        .alignment(ratatui::layout::Alignment::Left)
                        .wrap(Wrap { trim: true })
                };

                let block = ratatui::widgets::Block::default()
                    .title(title)
                    .borders(ratatui::widgets::Borders::ALL)
                    .style(ratatui::style::Style::default().fg(border_color));

                let layout = ratatui::layout::Layout::default()
                    .direction(ratatui::layout::Direction::Vertical)
                    .constraints(
                        [
                            ratatui::layout::Constraint::Length(1),
                            ratatui::layout::Constraint::Min(0),
                        ]
                        .as_ref(),
                    )
                    .split(block.inner(tui_area));
                frame.render_widget(&block, tui_area);
                frame.render_widget(&spinner_paragraph, layout[0]);
                frame.render_widget(&spans_paragraph, layout[1]);
                // Hint if we can't show everything on screen. This is a temporary solution
                // This overwrites the last line, which is very ugly, but it gets the job done avoiding some confusion for now.
                if spans_paragraph.line_count(layout[1].width) > layout[1].height as usize
                    && layout[1].height > 0
                {
                    let bottom = Rect {
                        x: layout[1].x,
                        y: layout[1].bottom() - 1,
                        width: layout[1].width,
                        height: 1,
                    };
                    frame.render_widget(
                        ratatui::widgets::Paragraph::new(
                            "... (more)                                                  ",
                        )
                        .style(ratatui::style::Style::default().fg(Color::Magenta))
                        .alignment(ratatui::layout::Alignment::Left),
                        bottom,
                    );
                }
            })),
        )?;
        self.tui_thread = Some(tui_thread);

        tracing::subscriber::set_global_default(logger).context("set_global_default")?;

        Ok(())
    }

    fn tear_down(&mut self) -> Result<()> {
        self.headless_logger.tear_down()?;

        // Restore stdout and stderr for direct use
        if let Some(stderr) = self.orig_stderr.as_ref() {
            dup2(stderr.as_raw_fd(), 2).context("tear_down: dup2 stderr")?;
            self.orig_stderr = None;
        }
        if let Some(stdout) = self.orig_stdout.as_ref() {
            dup2(stdout.as_raw_fd(), 1).context("tear_down: dup2 stdout")?;
            self.orig_stdout = None;
        }

        // Stop the reader thread
        if let Some(reader_thread) = self.log_shovel_thread.take() {
            reader_thread.join().unwrap();
            self.log_shovel_thread = None;
        }
        // Stop the TUI thread
        if let Some(tui_thread) = self.tui_thread.take() {
            tui_thread.join().unwrap().unwrap();
            self.tui_thread = None;
        }
        Ok(())
    }
}

struct TuiState<W: Write> {
    terminal: Terminal<CrosstermBackend<io::BufWriter<W>>>,
    log_receiver: mpsc::Receiver<String>,
    width: u16,
    height: u16,
    /// Number of lines in the drawn TUI area. 0 if not drawn yet.
    rendered_height: u16,
    graphics_mode: String,
}
impl<W: Write> TuiState<W> {
    fn new(log_receiver: mpsc::Receiver<String>, writer: W) -> Result<Self> {
        let (width, height) = terminal::size()?;
        let backend = CrosstermBackend::new(io::BufWriter::new(writer));
        let tui_height = Self::compute_tui_height_from_height(height);
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: Viewport::Fixed(Rect {
                    x: 0,
                    y: height - tui_height,
                    width,
                    height: tui_height,
                }),
            },
        )
        .context("initializing ratatui Terminal")?;
        Ok(Self {
            log_receiver,
            terminal,
            width,
            height,
            rendered_height: 0,
            graphics_mode: String::new(),
        })
    }
    fn compute_tui_height_from_height(height: u16) -> u16 {
        height / 3
    }
    fn compute_tui_height(&self) -> u16 {
        TuiState::<W>::compute_tui_height_from_height(self.height)
    }
    fn enable(&mut self) -> Result<()> {
        terminal::enable_raw_mode().context("terminal::enable_raw_mode")?;

        // Free up space at the bottom of the terminal for the TUI
        // It might be possible to read the current position to place the TUI
        // at a clever height, but that would be more involved (requiring reading
        // the cursor position)
        let h = self.compute_tui_height();
        for _ in 0..h {
            self.terminal.backend_mut().write(b"\r\n")?;
        }
        self.terminal.backend_mut().flush()?;
        // If the terminal had little content, we might not be at the TUI area yet
        self.terminal
            .backend_mut()
            .execute(cursor::MoveTo(0, self.height - h))?;
        self.rendered_height = h;

        Ok(())
    }
    fn run(
        &mut self,
        interrupt_state: InterruptState,
        render_callback: Arc<Box<dyn Fn(&mut Frame) + Send + Sync>>,
    ) -> Result<()> {
        let mut tui_height = self.compute_tui_height();
        let mut input_active = true;
        while input_active {
            // Re-fetch terminal size in case it was resized
            let (new_width, new_height) = terminal::size().unwrap();

            if new_width != self.width || new_height != self.height {
                let old_height = self.height;
                self.width = new_width;
                self.height = new_height;
                tui_height = self.compute_tui_height();
                let rect = Rect {
                    width: self.width as u16,
                    height: tui_height as u16,
                    x: 0,
                    y: self.height - tui_height,
                };
                self.terminal
                    .backend_mut()
                    .execute(cursor::MoveTo(0, self.height - 1))?;
                if old_height < new_height {
                    // This is probably dependent on terminal emulator specifics,
                    // but if the terminal window is grown vertically and the
                    // emulator keeps the bottom line attached to the bottom of
                    // the screen (typically when it has scrollback to be shown
                    // at the top), then the TUI area would line up exactly with
                    // the new location.
                    // However, in this branch we know that we've also increased
                    // the TUI height, so to avoid clobbering the logs that have
                    // been written in the TUI extension area, we need to scroll
                    // the terminal so that the logs are above the _new_ TUI
                    // area.
                    // If the TUI area is shrunk, we could potentially scroll
                    // so that we don't leave empty lines or garbage lines, but
                    // that runs the risk of accidentally overwriting logs.
                    // We prefer harmless garbage over lost logs.
                    for _ in old_height..new_height {
                        self.terminal.backend_mut().write(b"\r\n")?;
                    }
                    self.terminal.backend_mut().flush()?;
                }
                self.terminal.resize(rect).context("terminal.resize")?;
            }

            // Get all available log messages from the queue
            // This is a non-blocking operation
            let new_logs = {
                let mut new_logs = Vec::new();
                loop {
                    let r = self.log_receiver.try_recv();
                    match r {
                        Ok(log) => {
                            new_logs.push(log);
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            break;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            input_active = false;
                            break;
                        }
                    }
                }
                new_logs
            };

            // Handle log updates by reading from the log queue
            if !new_logs.is_empty() {
                let tui_start = self.height - tui_height;

                // Clear the TUI area before printing the logs
                for i in 0..tui_height {
                    self.terminal
                        .backend_mut()
                        .execute(cursor::MoveTo(0, tui_start + i))?;
                    self.terminal.backend_mut().execute(CLEAR_LINE)?;
                }
                // Move back to the end of the logging area, where logging will continue
                self.terminal
                    .backend_mut()
                    .execute(cursor::MoveTo(0, tui_start))?;

                // Print the log lines.
                // The first few lines will overwrite the TUI area; the rest will cause the terminal to scroll.
                self.terminal
                    .backend_mut()
                    .write(self.graphics_mode.as_bytes())
                    .unwrap();
                for log in new_logs {
                    self.terminal.backend_mut().write(b"nixops| ").unwrap();
                    self.terminal
                        .backend_mut()
                        .write(log.replace("\n", "\r\n").as_bytes())
                        .unwrap();
                    self.terminal.backend_mut().write(b"\r\n").unwrap();
                    save_color(log.as_str(), &mut self.graphics_mode);
                }

                // Cause the terminal to scroll so that the TUI area fits on screen
                for _ in 1..tui_height {
                    self.terminal.backend_mut().write(b"\r\n").unwrap();
                }
                self.rendered_height = tui_height;

                // Prepare for redraw
                self.terminal.backend_mut().flush()?;
                self.terminal.clear().unwrap();
            }

            self.terminal
                .backend_mut()
                .execute(cursor::MoveTo(0, self.height - tui_height))?;
            // Redraw the TUI at the bottom
            self.terminal
                .draw(render_callback.as_ref())
                .expect("terminal.draw");
            self.rendered_height = tui_height;

            // Check for user input
            if event::poll(Duration::from_millis(125))? {
                match event::read()? {
                    event::Event::Key(key) => {
                        match key.code {
                            KeyCode::Char('q') => {
                                interrupt_state.set_interrupted();
                            }
                            // Ctrl+C   (in raw mode, this is not a SIGINT)
                            KeyCode::Char('c')
                                if key.modifiers.contains(event::KeyModifiers::CONTROL) =>
                            {
                                interrupt_state.set_interrupted();
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn disable(&mut self) -> Result<()> {
        // We're done!
        // Clear the TUI area before exiting
        let tui_start = self.height - self.rendered_height;
        self.terminal
            .backend_mut()
            .execute(cursor::MoveTo(0, tui_start))?;
        for i in 0..self.rendered_height {
            self.terminal
                .backend_mut()
                .execute(cursor::MoveTo(0, tui_start + i))?;
            // UntilNewLine has better reflowing behavior than CurrentLine
            self.terminal.backend_mut().execute(CLEAR_LINE)?;
        }
        // Move back to the end of the logging area, where logging or shell session will continue
        self.terminal
            .backend_mut()
            .execute(cursor::MoveTo(0, tui_start))?;

        terminal::disable_raw_mode().context("disable_raw_mode")
    }
}

// ClearType::CurrentLine makes the terminal behave as if spaces were written
// across the whole line, and these spaces are reflowed when the terminal is
// resized. This would lead to many empty lines appearing in the log, instead of
// the normal text reflowing behavior.
const CLEAR_LINE: crossterm::terminal::Clear = terminal::Clear(ClearType::UntilNewLine);

fn spawn_log_ui<W: Write + Send + 'static>(
    interrupt_state: InterruptState,
    writer: W,
    log_receiver: mpsc::Receiver<String>,
    render_callback: Arc<Box<dyn Fn(&mut Frame) + Send + Sync>>,
) -> Result<thread::JoinHandle<Result<()>>, anyhow::Error> {
    let mut tui_state = TuiState::new(log_receiver, writer)?;
    Ok(thread::spawn(move || {
        tui_state.enable()?;
        tui_state.run(interrupt_state, render_callback)?;
        tui_state.disable()?;
        Ok(())
    }))
}

fn save_color(log: &str, graphics_mode: &mut String) {
    let parsed = ansi_parser::AnsiParser::ansi_parse(log);

    for item in parsed {
        match item {
            ansi_parser::Output::TextBlock(_) => {}
            ansi_parser::Output::Escape(e) => {
                // We ignore reverse video because it's not reliably emulated.
                // (https://en.wikipedia.org/wiki/ANSI_escape_code)
                match e {
                    ansi_parser::AnsiSequence::SetGraphicsMode(_) => {
                        let s = e.to_string();
                        *graphics_mode = s;
                    }
                    _ => {}
                }
            }
        }
    }
}

/// A `tracing_subscriber` layer that maintains a set of IDs of active spans.
/// The library does not seem to offer this information by itself, and we don't
/// want to track all spans in the end; just the ones that we may want to show.
struct SpanCollector {
    active_spans: Arc<Mutex<BTreeSet<u64>>>,
}
impl SpanCollector {
    fn new(active_spans: Arc<Mutex<BTreeSet<u64>>>) -> Self {
        Self { active_spans }
    }
}
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SpanCollector {
    fn on_new_span(
        &self,
        _span: &tracing::span::Attributes,
        id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        self.active_spans.lock().unwrap().insert(id.into_u64());
    }
    fn on_close(&self, id: tracing::Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        self.active_spans.lock().unwrap().remove(&id.into_u64());
    }
}
