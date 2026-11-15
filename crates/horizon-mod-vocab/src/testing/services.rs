//! The service fakes: the commands a mod runs and the messages it sends.
use crate::{CommandResult, Commands, Messages, Text};
use std::cell::RefCell;
use std::collections::HashSet;

// no wire: in-process testing harness state, not a mod boundary payload.
/// The server's command service: the commands run, in order, of which those listed as known
/// succeed, and the feedback given.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeCommands {
    known: HashSet<String>,
    /// Each command line run, in order.
    pub ran: Vec<String>,
    /// Each feedback line given, in order.
    pub feedback: Vec<Text>,
}

impl FakeCommands {
    /// Makes command line `line` succeed when run.
    pub fn known(&mut self, line: &str) -> &mut Self {
        self.known.insert(line.to_owned());
        self
    }
}

impl Commands for FakeCommands {
    fn run_command(&mut self, line: &str) -> bool {
        self.ran.push(line.to_owned());
        self.known.contains(line)
    }
    fn run_command_with_output(&mut self, line: &str) -> CommandResult {
        CommandResult::new(self.run_command(line), Vec::new())
    }
    fn feedback(&mut self, line: Text) -> Result<(), crate::text::TextFormError> {
        line.validate_depth()?;
        self.feedback.push(line);
        Ok(())
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// The server's message service: the lines broadcast, in order, and the texts the chat filter
/// blocks.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeMessages {
    blocked: HashSet<String>,
    broadcast: RefCell<Vec<Text>>,
}

impl FakeMessages {
    /// Makes the chat filter block text `text`.
    pub fn blocked(&mut self, text: &str) -> &mut Self {
        self.blocked.insert(text.to_owned());
        self
    }
    /// Each line broadcast, in order.
    pub fn broadcast_lines(&self) -> Vec<Text> {
        self.broadcast.borrow().clone()
    }
}

impl Messages for FakeMessages {
    fn broadcast(&self, line: Text) -> Result<(), crate::text::TextFormError> {
        line.validate_depth()?;
        self.broadcast.borrow_mut().push(line);
        Ok(())
    }
    fn text_allowed(&self, text: &str) -> Option<bool> {
        Some(!self.blocked.contains(text))
    }
}
