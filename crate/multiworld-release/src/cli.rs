use {
    std::{
        collections::BTreeMap,
        fmt,
        io::{
            self,
            Stdout,
            stdout,
        },
        sync::Arc,
    },
    chrono::prelude::*,
    crossterm::{
        cursor::MoveToColumn,
        style::Print,
        terminal::{
            Clear,
            ClearType,
            disable_raw_mode,
            enable_raw_mode,
        },
    },
    log_lock::*,
};

pub(crate) struct Cli {
    stdout: Arc<Mutex<Stdout>>,
    active_tasks: Arc<Mutex<BTreeMap<String, String>>>,
}

impl Cli {
    pub(crate) fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self {
            stdout: Arc::new(Mutex::new(stdout())),
            active_tasks: Arc::default(),
        })
    }

    pub(crate) async fn new_line(&self, initial_text: impl fmt::Display) -> io::Result<LineHandle> {
        {
            let mut stdout = lock!(self.stdout);
            crossterm::execute!(stdout,
                Print(format_args!("{} {initial_text}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
            )?;
        }
        Ok(LineHandle { stdout: Arc::clone(&self.stdout) })
    }

    pub(crate) async fn run<T>(&self, mut task: impl gres::Task<T> + fmt::Display, prefix: impl fmt::Display) -> io::Result<T> {
        let prefix = prefix.to_string();
        lock!(self.active_tasks).insert(prefix.clone(), task.to_string());
        {
            let mut stdout = lock!(self.stdout);
            crossterm::execute!(stdout,
                MoveToColumn(0),
                Clear(ClearType::UntilNewLine),
                Print(format_args!("{} {prefix:>26}: {task}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
            )?;
        }
        self.redraw().await?;
        loop {
            match task.run().await {
                Ok(result) => {
                    lock!(self.active_tasks).remove(&prefix);
                    {
                        let mut stdout = lock!(self.stdout);
                        crossterm::execute!(stdout,
                            MoveToColumn(0),
                            Clear(ClearType::UntilNewLine),
                            Print(format_args!("{} {prefix:>26}: done\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
                        )?;
                    }
                    self.redraw().await?;
                    break Ok(result)
                }
                Err(next_task) => {
                    task = next_task;
                    *lock!(self.active_tasks).get_mut(&prefix).expect("missing task, did you run multiple tasks with the same prefix?") = task.to_string();
                    {
                        let mut stdout = lock!(self.stdout);
                        crossterm::execute!(stdout,
                            MoveToColumn(0),
                            Clear(ClearType::UntilNewLine),
                            Print(format_args!("{} {prefix:>26}: {task}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
                        )?;
                    }
                    self.redraw().await?;
                }
            }
        }
    }

    async fn redraw(&self) -> io::Result<()> {
        let active_tasks = lock!(self.active_tasks);
        let mut stdout = lock!(self.stdout);
        let mut active_tasks = active_tasks.iter();
        if let Some((prefix, task)) = active_tasks.next() { //TODO sort by priority (active work preferred over waiting)
            if active_tasks.next().is_some() {
                crossterm::execute!(stdout,
                    MoveToColumn(0),
                    Clear(ClearType::UntilNewLine),
                    Print(format_args!("{} tasks, e.g.: {prefix}: {task}", active_tasks.len())),
                )?;
            } else {
                crossterm::execute!(stdout,
                    MoveToColumn(0),
                    Clear(ClearType::UntilNewLine),
                    Print(format_args!("1 task: {prefix}: {task}")),
                )?;
            }
        } else {
            crossterm::execute!(stdout,
                MoveToColumn(0),
                Clear(ClearType::UntilNewLine),
            )?;
        }
        Ok(())
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
            Print(format_args!("{} {new_text}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
        )?;
        Ok(())
    }
}
