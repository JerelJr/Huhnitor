use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use rustyline::{Cmd, KeyCode, KeyEvent, Modifiers};

use crate::error;

pub fn receiver(sender: UnboundedSender<String>) {
    let mut rl = rustyline::DefaultEditor::new().expect("Unable to start command history");
    rl.bind_sequence(KeyEvent(KeyCode::Up, Modifiers::empty()), Cmd::LineUpOrPreviousHistory(1));
    rl.bind_sequence(KeyEvent(KeyCode::Down, Modifiers::empty()), Cmd::LineDownOrNextHistory(1));

    match rl.readline(">> ") {
        Ok(line) => {
            rl.add_history_entry(&line).expect("Unable to add history entry");
            if sender.send(format!("{}\r\n", line.clone())).is_err() {
                error!("Couldn't report input to main thread!");
            }
        }
        Err(rustyline::error::ReadlineError::Interrupted) => {
            sender.send("stop\n".to_string()).expect("Couldn't stop!");
        }
        Err(e) => error!(e)
    }
}

pub async fn read_line(receiver: &mut UnboundedReceiver<String>) -> Option<String> {
    Some(receiver.recv().await?.trim().to_string())
}
