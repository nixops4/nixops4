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
    widgets::Paragraph,
    Frame, Terminal, Viewport,
};
use std::{
    fs::File,
    io::{self, BufRead as _, Write},
    os::fd::{AsRawFd as _, FromRawFd},
    sync::{mpsc, Arc},
    thread,
    time::Duration,
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

                // TODO: Show current activites
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

                let paragraph = Paragraph::new(text)
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

                let block = ratatui::widgets::Block::default()
                    .title(title)
                    .borders(ratatui::widgets::Borders::ALL)
                    .style(ratatui::style::Style::default().fg(border_color));
                frame.render_widget(paragraph.clone().block(block), tui_area)
            })),
        )?;
        self.tui_thread = Some(tui_thread);

        self.headless_logger.set_up(options)?;

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
    graphics_mode: String,
}
impl<W: Write> TuiState<W> {
    fn new(log_receiver: mpsc::Receiver<String>, writer: W) -> Result<Self> {
        let (width, height) = terminal::size()?;
        let backend = CrosstermBackend::new(io::BufWriter::new(writer));
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: Viewport::Fixed(Rect {
                    x: 0,
                    y: height - height / 3,
                    width: width - 0,
                    height: height / 3,
                }),
            },
        )
        .context("initializing ratatui Terminal")?;
        Ok(Self {
            log_receiver,
            terminal,
            width,
            height,
            graphics_mode: String::new(),
        })
    }
    fn enable(&mut self) -> Result<()> {
        terminal::enable_raw_mode().context("terminal::enable_raw_mode")
    }
    fn run(
        &mut self,
        interrupt_state: InterruptState,
        render_callback: Arc<Box<dyn Fn(&mut Frame) + Send + Sync>>,
    ) -> Result<()> {
        let mut tui_height = self.height / 3;
        let mut input_active = true;
        while input_active {
            // Re-fetch terminal size in case it was resized
            let (new_width, new_height) = terminal::size().unwrap();
            let tui_start = self.height - tui_height;
            if new_width != self.width || new_height != self.height {
                self.width = new_width;
                self.height = new_height;
                tui_height = self.height / 3;
                let rect = Rect {
                    width: self.width as u16,
                    height: tui_height as u16,
                    x: 0,
                    y: self.height - tui_height,
                };
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
                // Clear the TUI area before printing the logs
                for i in 0..tui_height {
                    self.terminal
                        .backend_mut()
                        .execute(cursor::MoveTo(0, tui_start + i))?;
                    // UntilNewLine has better reflowing behavior than CurrentLine
                    self.terminal
                        .backend_mut()
                        .execute(terminal::Clear(ClearType::UntilNewLine))?;
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
                    self.terminal
                        .backend_mut()
                        .write(log.replace("\n", "\r\n").as_bytes())
                        .unwrap();
                    self.terminal.backend_mut().write(b"\r\n").unwrap();
                    save_color(log.as_str(), &mut self.graphics_mode);
                }

                // Create/"scroll" the TUI area before redrawing it
                for _ in 1..tui_height {
                    self.terminal.backend_mut().write(b"\r\n").unwrap();
                }

                // Prepare for redraw
                self.terminal.clear().unwrap();
            }

            // Redraw the TUI at the bottom
            self.terminal
                .draw(render_callback.as_ref())
                .expect("terminal.draw");

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
        // Clear the TUI area before exiting, and move the cursor to the bottom of the log area
        // TODO: dedup with TUI clearing code when logging
        let tui_height = self.height / 3; // FIXME use saved height
        let tui_start = self.height - tui_height;
        for i in 0..tui_height {
            self.terminal
                .backend_mut()
                .execute(cursor::MoveTo(0, tui_start + i))?;
            // UntilNewLine has better reflowing behavior than CurrentLine
            self.terminal
                .backend_mut()
                .execute(terminal::Clear(ClearType::UntilNewLine))?;
        }
        // Move back to the end of the logging area, where logging will continue
        self.terminal
            .backend_mut()
            .execute(cursor::MoveTo(0, tui_start))?;

        // Clean up terminal when exiting
        terminal::disable_raw_mode().context("disable_raw_mode")
    }
}

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
