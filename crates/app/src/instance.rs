//! One PolishCast at a time. Closing the window only hides it (the app stays in the menu bar
//! or notification area, ready to record), so launching it again — from the Start menu, a
//! shortcut — would otherwise start a second, hidden copy each time. macOS reopens the running
//! app by itself; Windows and Linux don't.
//!
//! The running instance holds an exclusive lock on a file in the app's data folder and listens
//! on a local port, written beside it. A later launch finds the lock taken, asks that instance
//! to show its window, and exits. `polishcast mcp` asks it to record on the same port (`remote`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// What a later launch asks for.
const SHOW: &str = "show";
/// The longest request read. Any local program can connect: it must not be able to make the
/// app hold all it sends. A recording request is a few hundred bytes of JSON.
const MAX_REQUEST: u64 = 64 * 1024;

/// Set when a later launch asked for the window; the app shows it and clears it.
static SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether this process goes on to open the window.
pub enum Claim {
    /// The only one running: it keeps the lock (when it could take one) for as long as it lives.
    First(Option<std::fs::File>),
    /// Another instance runs and was asked to show itself.
    AlreadyRunning,
}

/// Takes the single-instance lock in `dir`, or hands over to the instance that holds it.
pub fn claim(dir: &Path) -> Claim {
    let _ = std::fs::create_dir_all(dir);
    let lock_path = dir.join("instance.lock");
    let port_path = dir.join("instance.port");
    let Ok(lock) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path) else {
        // No lock to be had (a read-only folder): better a second copy than no window.
        return Claim::First(None);
    };
    match lock.try_lock() {
        Ok(()) => {
            listen(&port_path);
            Claim::First(Some(lock))
        }
        Err(_) => {
            if ask_to_show(&port_path) {
                Claim::AlreadyRunning
            } else {
                // Locked, yet nobody answers: open rather than leave the person with nothing.
                Claim::First(None)
            }
        }
    }
}

/// Listens on a local port for later launches, and writes the port where they find it.
fn listen(port_path: &Path) {
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else { return };
    let Ok(addr) = listener.local_addr() else { return };
    if std::fs::write(port_path, addr.port().to_string()).is_err() {
        return;
    }
    // Without a token nothing can be asked to record; showing the window still works.
    let token = port_path.parent().and_then(crate::remote::write_token).unwrap_or_default();
    std::thread::Builder::new()
        .name("single-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let token = token.clone();
                // Its own thread: stopping a recording answers once the take is saved.
                std::thread::spawn(move || answer(stream, &token));
            }
        })
        .ok();
}

fn answer(mut stream: TcpStream, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut line = String::new();
    let Ok(clone) = stream.try_clone() else { return };
    let Ok(n) = BufReader::new(clone.take(MAX_REQUEST)).read_line(&mut line) else { return };
    if n as u64 == MAX_REQUEST && !line.ends_with('\n') {
        return;
    }
    if line.trim() == SHOW {
        SHOW_REQUESTED.store(true, Ordering::SeqCst);
    } else if !token.is_empty() {
        let mut reply = serde_json::to_vec(&crate::remote::answer(line.trim(), token)).unwrap_or_default();
        reply.push(b'\n');
        let _ = stream.write_all(&reply);
    }
}

/// Asks the running instance to show its window; whether it got the message.
fn ask_to_show(port_path: &Path) -> bool {
    let Some(port) = std::fs::read_to_string(port_path).ok().and_then(|p| p.trim().parse::<u16>().ok()) else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(2)) else {
        return false;
    };
    stream.write_all(format!("{SHOW}\n").as_bytes()).is_ok()
}

/// Whether a later launch asked for the window since last asked; clears the request.
pub fn take_show_request() -> bool {
    SHOW_REQUESTED.swap(false, Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sends `request` to `answer` over a real local connection and returns what came back.
    fn ask(request: Vec<u8>) -> String {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || answer(listener.accept().unwrap().0, "secret"));
        let mut client = TcpStream::connect(addr).unwrap();
        // The server may stop reading before all of it is sent.
        let _ = client.write_all(&request);
        let _ = client.shutdown(std::net::Shutdown::Write);
        let mut reply = String::new();
        let _ = client.read_to_string(&mut reply);
        server.join().unwrap();
        reply
    }

    #[test]
    fn requests_past_the_limit_are_not_read_to_the_end() {
        assert!(ask(b"{}\n".to_vec()).contains("not allowed"));
        let mut huge = vec![b'a'; 4 * MAX_REQUEST as usize];
        huge.push(b'\n');
        assert_eq!(ask(huge), "", "a request past the limit was answered");
    }

    #[test]
    fn a_second_launch_hands_over_to_the_first() {
        let dir = std::env::temp_dir().join(format!("polishcast-instance-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = claim(&dir);
        assert!(matches!(first, Claim::First(_)));
        assert!(!take_show_request());

        // The lock is per open file, so a second claim in the same process sees it taken.
        assert!(matches!(claim(&dir), Claim::AlreadyRunning));
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !SHOW_REQUESTED.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(take_show_request());
        assert!(!take_show_request());

        // Once the first is gone, the next launch is the first again.
        drop(first);
        assert!(matches!(claim(&dir), Claim::First(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
