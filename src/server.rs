use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

use futures_channel::mpsc::UnboundedSender;
use log::{debug, error};

const PORT: u16 = 8080;

const PAGE: &[u8] = br#"<!doctype html>
<html>
<head><meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no">
<style>
  html,body{margin:0;height:100%;overflow:hidden}
  button{width:100vw;height:100dvh;border:none;background:green}
  button:active,button.on{background:red}
</style>
</head>
<body>
  <button id="btn"></button>
  <script>
    const toggle = new URLSearchParams(location.search).get('mode') === 'toggle';
    document.getElementById('btn').addEventListener('touchstart', e => {
      e.preventDefault();
      fetch(location.href, {method: 'POST'});
      if (toggle) e.target.classList.toggle('on');
    });
  </script>
</body>
</html>
"#;

pub fn spawn(action_tx: UnboundedSender<String>) {
    thread::spawn(move || {
        let listener = match TcpListener::bind(("0.0.0.0", PORT)) {
            Ok(l) => l,
            Err(e) => {
                error!(target: "server", "state=error reason={e}");
                return;
            }
        };
        debug!(target: "server", "state=listening port={PORT}");
        for stream in listener.incoming().flatten() {
            handle_connection(stream, &action_tx);
        }
    });
}

fn handle_connection(mut stream: TcpStream, action_tx: &UnboundedSender<String>) {
    let mut request_line = String::new();
    if BufReader::new(&stream)
        .read_line(&mut request_line)
        .is_err()
    {
        return;
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();

    if method == "POST" {
        for action in parse_actions(path) {
            let _ = action_tx.unbounded_send(action);
        }
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
    } else {
        let _ = stream.write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                PAGE.len()
            )
            .as_bytes(),
        );
        let _ = stream.write_all(PAGE);
    }
}

fn parse_actions(path: &str) -> Vec<String> {
    let Some((_, query)) = path.split_once('?') else {
        return Vec::new();
    };
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(key, _)| *key == "action")
        .map(|(_, value)| value.to_string())
        .collect()
}
