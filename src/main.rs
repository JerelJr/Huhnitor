use crate::app::App;
use handler::handle;
use std::env;
use std::time::Duration;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use structopt::StructOpt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

mod app;
#[macro_use]
mod handler;
mod input;
mod output;
mod port;

async fn monitor(
    cmd_port: Option<String>,
    auto: bool,
    no_welcome: bool,
    out: &output::Preferences,
    app: App,
) {
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let (output_tx, output_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let input_clone = input_tx.clone();

    std::thread::spawn(|| input::receiver(input_clone));


    let tty_path = if cmd_port.is_some() {
        cmd_port
    } else if auto {
        port::auto(&mut input_rx, out).await
    } else {
        port::manual(&mut input_rx, out).await
    };

    if let Some(inner_tty_path) = tty_path {
        let settings = tokio_serial::new(&inner_tty_path, 115200).data_bits(DataBits::Eight).flow_control(FlowControl::None).parity(Parity::None).stop_bits(StopBits::One).timeout(Duration::from_secs(10));
        #[allow(unused_mut)] // Ignore warning from windows compilers
        if let Ok(mut port) = tokio_serial::SerialStream::open(&settings) {
            #[cfg(unix)]
            port.set_exclusive(false)
                .expect("Unable to set serial port exclusive to false");

            let mut port = BufReader::new(port);

            out.connected(&inner_tty_path);

            if !no_welcome {
                if let Err(_) = port.write("welcome\r\n".as_bytes()).await {
                    out.print("Couldn't send welcome command!");
                }
            }

            tokio::spawn(async move { app.run(input_tx, output_rx, Duration::from_millis(15)).await });

            let mut buf = Vec::new();
            loop {
                tokio::select! {
                    len = port.read_until(b'\n', &mut buf) => match len {
                        Ok(0) => { // EOF
                            break;
                        },
                        Ok(_) => {
                            let input = String::from_utf8_lossy(&buf).to_string();
                            output_tx.send(input).unwrap();
                            buf = Vec::new();
                        },
                        Err(e) => {
                            error!(e);
                            break;
                        }
                    },

                    Some(text) = input_rx.recv() => {
                        if text.trim().to_uppercase() == "EXIT" {
                            break;
                        } else if text.trim().to_uppercase() == "CLEAR" {
                            output::clear();
                        } else if text.to_uppercase().starts_with("HUHN") {
                            if port.write(handle(text).as_bytes()).await.is_err() {
                                error!("Command failed");
                            }
                        } else if port.write(text.as_bytes()).await.is_err() {
                            error!("Couldn't send message");
                        }
                    }
                }
            }
        } else {
            // Port creation handler
            error!("Couldn't create port object!");
        }
    } else {
        // Path handler
        out.hint();
    }
}

#[derive(StructOpt)]
#[structopt(name = "Huhnitor", about = env!("CARGO_PKG_DESCRIPTION"))]
struct Opt {
    /// Open driver page
    #[structopt(short, long)]
    driver: bool,

    /// Disable automatic port connection
    #[structopt(short = "a", long = "no-auto")]
    auto: bool,

    /// Disable colored output
    #[structopt(short = "c", long = "no-color")]
    color: bool,

    /// Select port
    #[structopt(short, long)]
    port: Option<String>,

    /// Disable welcome command
    #[structopt(short = "w", long = "no-welcome")]
    no_welcome: bool,
}

#[tokio::main]
async fn main() {
    let args = Opt::from_args();

    let out = output::Preferences {
        color_enabled: !args.color,
    };

    out.logo();
    out.version();

    if args.driver {
        out.driver();
    } else {
        let mut app = app::App::new();
        monitor(args.port, !args.auto, args.no_welcome, &out, app).await;
    }

    out.goodbye();
}
