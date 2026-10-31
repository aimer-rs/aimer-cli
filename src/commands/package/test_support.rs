//! A [`Reporter`] for the package tests that records the steps it is asked to
//! run instead of touching the machine.

use std::process::Command;
use std::sync::Mutex;

use crate::commands::assemble::{Reporter, Step};

/// One recorded step: the program, its arguments and the action it carried out.
#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub program: String,
    pub args: Vec<String>,
    pub action: String,
}

pub(super) struct SpyReporter {
    steps: Mutex<Vec<Recorded>>,
    notes: Mutex<Vec<String>>,
    fail_on: Option<&'static str>,
    /// Stands in for what a real tool leaves on disk.
    on_run: Option<Box<dyn Fn(&Command) + Send + Sync>>,
}

impl SpyReporter {
    pub(super) fn new() -> Self {
        Self {
            steps: Mutex::new(Vec::new()),
            notes: Mutex::new(Vec::new()),
            fail_on: None,
            on_run: None,
        }
    }

    /// A spy that also calls `hook` with every command, for steps whose result
    /// later code looks for on disk.
    pub(super) fn with_hook(hook: impl Fn(&Command) + Send + Sync + 'static) -> Self {
        Self {
            on_run: Some(Box::new(hook)),
            ..Self::new()
        }
    }

    /// A spy whose step with this action fails (after being recorded).
    pub(super) fn failing(action: &'static str) -> Self {
        Self {
            fail_on: Some(action),
            ..Self::new()
        }
    }

    /// The same spy, with the step of this action failing (after being
    /// recorded).
    pub(super) fn failing_on(mut self, action: &'static str) -> Self {
        self.fail_on = Some(action);
        self
    }

    pub(super) fn steps(&self) -> Vec<Recorded> {
        self.steps.lock().unwrap().clone()
    }

    pub(super) fn programs(&self) -> Vec<String> {
        self.steps().into_iter().map(|s| s.program).collect()
    }

    pub(super) fn notes(&self) -> Vec<String> {
        self.notes.lock().unwrap().clone()
    }
}

impl Reporter for SpyReporter {
    fn note(&self, message: String) {
        self.notes.lock().unwrap().push(message);
    }

    fn run(&self, cmd: Command, step: Step) -> anyhow::Result<()> {
        if let Some(hook) = &self.on_run {
            hook(&cmd);
        }
        self.steps.lock().unwrap().push(Recorded {
            program: cmd.get_program().to_string_lossy().to_string(),
            args: args_of(&cmd),
            action: step.action.clone(),
        });
        if self.fail_on == Some(step.action.as_str()) {
            anyhow::bail!("{} failed", step.action);
        }
        Ok(())
    }
}

/// The arguments of `cmd`, as lossy strings.
pub(super) fn args_of(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect()
}
