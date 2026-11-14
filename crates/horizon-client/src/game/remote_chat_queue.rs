//! `ChatListener`'s delayed queue: Chat Delay's held-back messages, their
//! tick, the flush when the delay turns off, and the queue row's click.
use super::super::Game;
use std::time::Duration;

impl Game<'_> {
    /// `ChatListener.messageDelay`: Chat Delay in milliseconds, as Java
    /// truncates it.
    pub(super) fn chat_delay_ms(&self) -> i64 {
        (self.settings.chat_delay * 1000.0) as i64
    }

    /// `ChatListener.queueSize`: the messages Chat Delay holds, for the chat's
    /// `chat.queue` row.
    pub(in crate::game) fn chat_queue_len(&self) -> usize {
        self.remote.as_ref().map_or(0, |r| r.chat_queue.len())
    }

    /// `ChatListener.tick`: while paused the wait does not run down; with no
    /// Chat Delay the queue flushes; otherwise the next message the chat takes
    /// shows once the delay has passed since the last shown one.
    pub(in crate::game) fn tick_chat_queue(&mut self) {
        let delay = self.chat_delay_ms();
        // Minecraft.isPaused: a single-player pausing screen, not every pause menu.
        if self.pause {
            if delay > 0
                && let Some(r) = self.remote.as_mut()
            {
                r.chat_previous = r.chat_previous.map(|t| t + Duration::from_millis(50));
            }
            return;
        }
        if delay == 0 {
            if self
                .remote
                .as_ref()
                .is_some_and(|r| !r.chat_queue.is_empty())
            {
                self.flush_chat_queue();
            }
            return;
        }
        let ready = self.remote.as_ref().is_some_and(|r| {
            r.chat_previous
                .is_none_or(|t| t.elapsed() >= Duration::from_millis(delay as u64))
        });
        if !ready {
            return;
        }
        // A message the Chat option hides is dropped and the next one is tried,
        // until one shows.
        while let Some(chat) = self.remote.as_mut().and_then(|r| r.chat_queue.pop_front()) {
            if self.accept_chat(chat) {
                break;
            }
        }
    }

    /// `ChatListener.flushQueue`: every held-back message shows now, and the
    /// delay starts over.
    pub(super) fn flush_chat_queue(&mut self) {
        let Some(queue) = self
            .remote
            .as_mut()
            .map(|r| std::mem::take(&mut r.chat_queue))
        else {
            return;
        };
        for chat in queue {
            self.accept_chat(chat);
        }
        if let Some(r) = self.remote.as_mut() {
            r.chat_previous = None;
        }
    }

    /// `ChatListener.acceptNextDelayedMessage`: the queue row's click shows the
    /// next held-back message now, without moving the delay timer.
    pub(in crate::game) fn accept_next_delayed_message(&mut self) {
        if let Some(chat) = self.remote.as_mut().and_then(|r| r.chat_queue.pop_front()) {
            self.accept_chat(chat);
        }
    }
}
