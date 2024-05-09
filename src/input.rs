use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use std::collections::VecDeque;
use std::time::{Instant, Duration};
use rustyline::{Cmd, KeyCode, KeyEvent, Modifiers};

use crate::error;

pub fn receiver(sender: UnboundedSender<String>) {
    let mut exitspam: VecDeque<Instant> = VecDeque::with_capacity(3);

    let mut rl = rustyline::DefaultEditor::new().expect("Unable to start command history");
    rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::empty()), Cmd::LineUpOrPreviousHistory(1));
    rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::empty()), Cmd::LineDownOrNextHistory(1));

    match rl.readline(">> ") {
        Ok(line) => {
            rl.add_history_entry(&line).expect("TODO: panic message");
            if sender.send(format!("{}\r\n", line.clone())).is_err() {
                error!("Couldn't report input to main thread!");
            }

            if line.trim().to_uppercase() == "EXIT" {
                return;
            }
        }
        Err(rustyline::error::ReadlineError::Interrupted) => {
            sender.send("stop\n".to_string()).expect("Couldn't stop!");

            if exitspam.len() == 3 {
                if let Some(time) = exitspam.pop_back() {
                    if Instant::now() - time <= Duration::new(3, 0) {
                        sender.send("EXIT".to_string()).expect("Couldn't exit!");
                        return;
                    } else {
                        exitspam.push_front(Instant::now());
                    }
                }
            } else {
                exitspam.push_front(Instant::now());
            }
        }
        Err(e) => error!(e)
    }
}

pub async fn read_line(receiver: &mut UnboundedReceiver<String>) -> Option<String> {
    Some(receiver.recv().await?.trim().to_string())
}
