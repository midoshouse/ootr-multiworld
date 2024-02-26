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

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Priority {
    Waiting,
    Active,
    UserInput,
}

pub(crate) trait GetPriority {
    fn priority(&self) -> Priority;
}

pub(crate) struct Cli {
    stdout: Arc<Mutex<Stdout>>,
    active_tasks: Arc<Mutex<BTreeMap<String, (Priority, String)>>>,
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
        lock!(stdout = self.stdout; crossterm::execute!(stdout,
            Print(format_args!("{} {initial_text}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
        ))?;
        Ok(LineHandle { stdout: Arc::clone(&self.stdout) })
    }

    pub(crate) async fn run<T>(&self, mut task: impl gres::Task<T> + GetPriority + fmt::Display, prefix: impl fmt::Display) -> io::Result<T> {
        let prefix = prefix.to_string();
        lock!(active_tasks = self.active_tasks; active_tasks.insert(prefix.clone(), (task.priority(), task.to_string())));
        lock!(stdout = self.stdout; crossterm::execute!(stdout,
            MoveToColumn(0),
            Clear(ClearType::UntilNewLine),
            Print(format_args!("{} {prefix:>26}: {task}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
        ))?;
        self.redraw().await?;
        loop {
            match task.run().await {
                Ok(result) => {
                    lock!(active_tasks = self.active_tasks; active_tasks.remove(&prefix));
                    lock!(stdout = self.stdout; crossterm::execute!(stdout,
                        MoveToColumn(0),
                        Clear(ClearType::UntilNewLine),
                        Print(format_args!("{} {prefix:>26}: done\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
                    ))?;
                    self.redraw().await?;
                    break Ok(result)
                }
                Err(next_task) => {
                    task = next_task;
                    lock!(active_tasks = self.active_tasks; *active_tasks.get_mut(&prefix).expect("missing task, did you run multiple tasks with the same prefix?") = (task.priority(), task.to_string()));
                    lock!(stdout = self.stdout; crossterm::execute!(stdout,
                        MoveToColumn(0),
                        Clear(ClearType::UntilNewLine),
                        Print(format_args!("{} {prefix:>26}: {task}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
                    ))?;
                    self.redraw().await?;
                }
            }
        }
    }

    async fn redraw(&self) -> io::Result<()> {
        lock!(active_tasks = self.active_tasks; lock!(stdout = self.stdout; if let Some((prefix, (_, task))) = active_tasks.iter().min_by(|(prefix1, (priority1, _)), (prefix2, (priority2, _))| priority2.cmp(priority1).then_with(|| prefix1.cmp(prefix2))) { // max priority, then alphabetically
            if active_tasks.len() > 1 {
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
        }));
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
        lock!(stdout = self.stdout; crossterm::execute!(stdout,
            Print(format_args!("{} {new_text}\r\n", Local::now().format("%Y-%m-%d %H:%M:%S"))),
        ))?;
        Ok(())
    }
}
