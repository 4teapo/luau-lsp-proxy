use flate2::read::GzDecoder;
use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
};
use tiny_http::{Method, Response, Server, StatusCode};

fn serialize_notification(method: String, params: Option<String>) -> String {
    let content = format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"{}\",\"params\":{}}}",
        method,
        params.unwrap_or("null".into())
    );
    let header = format!("Content-Length: {}\r\n\r\n", content.len());
    format!("{}{}", header, content)
}

fn main() -> io::Result<()> {
    let args = std::env::args().collect::<Vec<String>>();
    let mut child = Command::new(&args[2])
        .args(&args[3..])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .spawn()?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    let mut child_stdin = child.stdin.take().expect("Failed to take child stdin");
    thread::spawn(move || {
        loop {
            let data = rx.recv().unwrap();
            child_stdin.write_all(data.as_slice()).unwrap();
            child_stdin.flush().unwrap();
        }
    });

    let tx1 = tx.clone();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin_lock = stdin.lock();
        let mut buffer = [0u8; 32768];
        loop {
            let n = stdin_lock.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            tx1.send(buffer[..n].to_vec()).unwrap();
        }
    });

    let server = Server::http(format!("127.0.0.1:{}", args[1])).expect("Failed to start server");

    let tx2 = tx.clone();
    thread::spawn(move || {
        for mut request in server.incoming_requests() {
            if *request.method() != Method::Post {
                request.respond(Response::empty(StatusCode(404))).ok();
                continue;
            }

            let mut ty: Option<&str> = None;
            let mut encoding: Option<&str> = None;

            for header in request.headers() {
                if header.field.equiv("content-type") {
                    ty = Some(header.value.as_str())
                } else if header.field.equiv("content-encoding") {
                    encoding = Some(header.value.as_str());
                }
            }

            if ty != Some("application/json") || encoding != Some("gzip") {
                request.respond(Response::empty(StatusCode(400))).ok();
                continue;
            }

            match request.url() {
                "/full" => {
                    let mut decompressor = GzDecoder::new(request.as_reader());
                    let mut body = String::new();
                    if decompressor.read_to_string(&mut body).is_err() {
                        request.respond(Response::empty(StatusCode(500))).ok();
                        continue;
                    }
                    let body = body[8..body.len() - 1].into();
                    let notification = serialize_notification("$/plugin/full".into(), Some(body));
                    if tx2.send(notification.into()).is_err() {
                        request.respond(Response::empty(StatusCode(500))).ok();
                        continue;
                    }
                    request.respond(Response::empty(StatusCode(200))).ok();
                }
                "/clear" => {
                    let notification = serialize_notification("$/plugin/clear".into(), None);
                    if tx2.send(notification.into()).is_err() {
                        request.respond(Response::empty(StatusCode(500))).ok();
                        continue;
                    }
                    request.respond(Response::empty(StatusCode(200))).ok();
                }
                _ => continue,
            };
        }
    });

    child.wait()?;

    Ok(())
}
