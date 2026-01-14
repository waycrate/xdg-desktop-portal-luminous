use dialoguer::FuzzySelect;
use dialoguer::theme::ColorfulTheme;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::exit;
use std::sync::mpsc::{Sender, channel};
use std::sync::{LazyLock, Mutex};
use std::thread;
use stream_message::{Request, Response, SERVER_SOCK, SocketMessage};

enum Selection {
    Accept(u32),
    Reject,
}

static ACTIVE_STREAM: LazyLock<Mutex<Option<Sender<Selection>>>> =
    LazyLock::new(|| Mutex::new(None));

use signal_hook::{consts::SIGINT, iterator::Signals};

fn handle_client(mut stream: UnixStream, osender: Sender<Vec<String>>) {
    let (sender, receiver) = channel();
    loop {
        let Ok(Request::ScreenShare { monitors }) = stream.read_msg() else {
            continue;
        };
        let Ok(mut the_sender) = ACTIVE_STREAM.try_lock() else {
            let _ = stream.write_msg(Response::Busy);
            continue;
        };
        if the_sender.is_some() {
            let _ = stream.write_msg(Response::Busy);
            continue;
        }
        *the_sender = Some(sender.clone());
        drop(the_sender);
        let _ = osender.send(monitors);

        match receiver.recv() {
            Ok(Selection::Accept(index)) => {
                let _ = stream.write_msg(Response::Success { index });
            }
            Ok(Selection::Reject) => {
                let _ = stream.write_msg(Response::Cancel);
            }
            Err(e) => {
                eprintln!("Error: {e}");
                let _ = stream.write_msg(Response::Busy);
            }
        }
    }
}

fn main() {
    let listener = UnixListener::bind(SERVER_SOCK.clone()).unwrap();
    let mut signals = Signals::new([SIGINT]).unwrap();
    let (sender, receiver) = channel();
    let handle = thread::spawn(move || {
        for sig in signals.forever() {
            if sig == SIGINT {
                let _ = std::fs::remove_file(SERVER_SOCK.clone());
                break;
            }
        }
        exit(0);
    });
    let stream_thread = thread::spawn(move || {
        let mut threads = vec![handle];
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                break;
            };
            let handler = thread::spawn({
                let sender = sender.clone();
                move || handle_client(stream, sender)
            });
            threads.push(handler);
        }
        for thread in threads {
            let _ = thread.join();
        }
    });

    while let Ok(monitor) = receiver.recv() {
        let select = FuzzySelect::with_theme(&ColorfulTheme::default())
            .with_prompt("select monitor")
            .default(0)
            .items(&monitor)
            .interact()
            .map(|index| Selection::Accept(index as u32))
            .unwrap_or(Selection::Reject);
        let mut active_stream = ACTIVE_STREAM.lock().expect("It should always alive");
        let stream = active_stream.as_ref().expect("should have one");
        let _ = stream.send(select);
        let _ = stream;
        *active_stream = None;
    }
    let _ = stream_thread.join();
}
