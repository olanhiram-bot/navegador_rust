//! Servidor HTTP local para los fixtures y para el "sink" que recibe las
//! peticiones exfiltradas. Dos puertos distintos para que el sink sea
//! cross-origin de verdad.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub body: String,
}

pub struct FixtureServer {
    page_port: u16,
    sink_port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    running: Arc<AtomicBool>,
}

impl FixtureServer {
    pub fn start() -> Self {
        let page_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind page");
        let sink_listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind sink");
        let page_port = page_listener.local_addr().unwrap().port();
        let sink_port = sink_listener.local_addr().unwrap().port();

        let running = Arc::new(AtomicBool::new(true));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let page_origin = format!("http://127.0.0.1:{page_port}");
        let sink_origin = format!("http://127.0.0.1:{sink_port}");

        {
            let running = running.clone();
            let fixtures = fixtures.clone();
            let page_origin = page_origin.clone();
            let sink_origin = sink_origin.clone();
            let requests = requests.clone();
            std::thread::spawn(move || {
                for stream in page_listener.incoming() {
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let _ = serve_page(stream, &fixtures, &page_origin, &sink_origin, &requests);
                }
            });
        }

        {
            let running = running.clone();
            let requests = requests.clone();
            std::thread::spawn(move || {
                for stream in sink_listener.incoming() {
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let _ = record_sink(stream, &requests);
                }
            });
        }

        FixtureServer {
            page_port,
            sink_port,
            requests,
            running,
        }
    }

    pub fn page_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.page_port)
    }

    pub fn sink_origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.sink_port)
    }

    pub fn page_url(&self, fixture: &str) -> String {
        format!("{}/{}", self.page_origin(), fixture)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }

    pub fn clear(&self) {
        self.requests.lock().unwrap().clear();
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        // Desbloquea los `accept`.
        let _ = TcpStream::connect(("127.0.0.1", self.page_port));
        let _ = TcpStream::connect(("127.0.0.1", self.sink_port));
    }
}

fn serve_page(
    mut stream: TcpStream,
    fixtures: &Path,
    page_origin: &str,
    sink_origin: &str,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream) else {
        return Ok(());
    };
    if request.method.eq_ignore_ascii_case("POST") {
        requests.lock().unwrap().push(RecordedRequest {
            method: request.method.clone(),
            path: request.path.clone(),
            body: request.body.clone(),
        });
    }
    let name = request.path.trim_start_matches('/');
    let name = if name.is_empty() { "legit.html" } else { name };
    if name.contains("..") {
        return respond(&mut stream, 400, "text/plain", "ruta inválida");
    }
    let path = fixtures.join(name);
    let body = match std::fs::read_to_string(&path) {
        Ok(body) => body
            .replace("{{PAGE}}", page_origin)
            .replace("{{SINK}}", sink_origin),
        Err(_) => {
            return respond(&mut stream, 404, "text/plain", "fixture no encontrado");
        }
    };
    respond(&mut stream, 200, "text/html; charset=utf-8", &body)
}

fn record_sink(
    mut stream: TcpStream,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) -> std::io::Result<()> {
    let Some(request) = read_request(&mut stream) else {
        return Ok(());
    };
    requests.lock().unwrap().push(RecordedRequest {
        method: request.method,
        path: request.path,
        body: request.body,
    });
    respond(&mut stream, 200, "text/html; charset=utf-8", "<h1>ok</h1>")
}

struct ParsedRequest {
    method: String,
    path: String,
    body: String,
}

fn read_request(stream: &mut TcpStream) -> Option<ParsedRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];

    let header_end = loop {
        if let Some(pos) = find(&buffer, b"\r\n\r\n") {
            break pos;
        }
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 64 * 1024 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let mut request_line = lines.next()?.split_whitespace();
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();

    let mut content_length = 0usize;
    for line in lines {
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().unwrap_or(0);
        }
    }

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Some(ParsedRequest {
        method,
        path,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}
