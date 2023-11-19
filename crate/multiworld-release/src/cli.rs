use {
    std::{
        fmt,
        io::{
            self,
            Stdout,
            stdout,
        },
        sync::Arc,
    },
    crossterm::{
        style::Print,
        terminal::{
            disable_raw_mode,
            enable_raw_mode,
        },
    },
    log_lock::*,
};

pub(crate) struct Cli {
    stdout: Arc<Mutex<Stdout>>,
}

impl Cli {
    pub(crate) fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self { stdout: Arc::new(Mutex::new(stdout())) })
    }

    pub(crate) async fn new_line(&self, initial_text: impl fmt::Display) -> io::Result<LineHandle> {
        {
            let mut stdout = lock!(self.stdout);
            crossterm::execute!(stdout,
                Print(initial_text),
            )?;
        }
        Ok(LineHandle { stdout: Arc::clone(&self.stdout) })
    }

    pub(crate) async fn run<T>(&self, mut task: impl gres::Task<T> + fmt::Display, done_label: impl fmt::Display) -> io::Result<T> {
        {
            let mut stdout = lock!(self.stdout);
            crossterm::execute!(stdout,
                Print(format_args!("[  0%] {task}\r\n")),
            )?;
        }
        loop {
            match task.run().await {
                Ok(result) => {
                    let mut stdout = lock!(self.stdout);
                    crossterm::execute!(stdout,
                        Print(format_args!("[done] {done_label}\r\n")),
                    )?;
                    break Ok(result)
                }
                Err(next_task) => {
                    task = next_task;
                    let mut stdout = lock!(self.stdout);
                    crossterm::execute!(stdout,
                        Print(format_args!("[{:>3}%] {task}\r\n", u8::from(task.progress()))),
                    )?;
                }
            }
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

#[non_exhaustive]
pub(crate) struct LineHandle {
    stdout: Arc<Mutex<Stdout>>,
}

impl LineHandle {
    pub(crate) async fn replace(&self, new_text: impl fmt::Display) -> io::Result<()> {
        let mut stdout = lock!(self.stdout);
        crossterm::execute!(stdout,
            Print(format_args!("\r{new_text}\r\n")),
        )?;
        Ok(())
    }
}
