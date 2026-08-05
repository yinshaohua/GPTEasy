use serde_json::Value;
use std::{
    env,
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {
            let port_file = PathBuf::from(args.next().ok_or("serve requires port file")?);
            let log_file = PathBuf::from(args.next().ok_or("serve requires log file")?);
            let max_requests: usize = args
                .next()
                .ok_or("serve requires max requests")?
                .parse()?;
            serve(&port_file, &log_file, max_requests)?;
        }
        Some("paths") => print_paths(),
        _ => {
            eprintln!("usage: codex-native-config-contract <serve PORT_FILE LOG_FILE MAX_REQUESTS|paths>");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn print_paths() {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let codex_home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| home.clone().map(|path| path.join(".codex")));
    let config = codex_home.as_ref().map(|path| path.join("config.toml"));
    let auth = codex_home.as_ref().map(|path| path.join("auth.json"));

    println!("platform={}", env::consts::OS);
    println!(
        "home={}",
        home.as_deref()
            .map(Path::display)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<unresolved>".to_string())
    );
    println!(
        "codex_home={}",
        codex_home
            .as_deref()
            .map(Path::display)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<unresolved>".to_string())
    );
    println!(
        "config_toml={}",
        config
            .as_deref()
            .map(Path::display)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<unresolved>".to_string())
    );
    println!(
        "auth_json={}",
        auth.as_deref()
            .map(Path::display)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<unresolved>".to_string())
    );
}

fn serve(
    port_file: &Path,
    log_file: &Path,
    max_requests: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = port_file.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = log_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    fs::write(port_file, port.to_string())?;

    let mut handled = 0usize;
    for connection in listener.incoming() {
        let mut stream = connection?;
        let request = read_http_request(&mut stream)?;
        let record = inspect_request(&request);
        append_json_line(log_file, &record)?;

        if record["path"] == "/v1/responses" && record["method"] == "POST" {
            let body = sse_body("spike-response-001", "GPTEasy mock response");
            write_response(
                &mut stream,
                "200 OK",
                "text/event-stream",
                &body,
                true,
            )?;
            handled += 1;
        } else {
            let body = r#"{"error":{"message":"not found"}}"#;
            write_response(&mut stream, "404 Not Found", "application/json", body, false)?;
        }

        if handled >= max_requests {
            break;
        }
    }
    Ok(())
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
    let mut data = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut header_end = None;
    let mut expected_body = 0usize;

    loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..read]);

        if header_end.is_none() {
            if let Some(index) = find_subslice(&data, b"\r\n\r\n") {
                header_end = Some(index + 4);
                let header = String::from_utf8_lossy(&data[..index]);
                for line in header.lines() {
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            expected_body = value.trim().parse().unwrap_or(0);
                        }
                    }
                }
            }
        }

        if let Some(end) = header_end {
            if data.len() >= end + expected_body {
                break;
            }
        }
    }
    Ok(data)
}

fn inspect_request(data: &[u8]) -> Value {
    let header_end = find_subslice(data, b"\r\n\r\n").unwrap_or(data.len());
    let header = String::from_utf8_lossy(&data[..header_end]);
    let first_line = header.lines().next().unwrap_or_default();
    let mut first_parts = first_line.split_whitespace();
    let method = first_parts.next().unwrap_or_default();
    let path = first_parts.next().unwrap_or_default();

    let authorization = header
        .lines()
        .find_map(|line| line.strip_prefix("Authorization:"))
        .or_else(|| {
            header
                .lines()
                .find_map(|line| line.strip_prefix("authorization:"))
        })
        .map(str::trim)
        .unwrap_or_default();
    let body = data
        .get(header_end.saturating_add(4)..)
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());

    serde_json::json!({
        "timestamp_unix": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
        "method": method,
        "path": path,
        "authorization_present": !authorization.is_empty(),
        "authorization_bearer": authorization.to_ascii_lowercase().starts_with("bearer "),
        "authorization_fingerprint": fingerprint(authorization),
        "model": body.as_ref().and_then(|value| value.get("model")).and_then(Value::as_str),
        "stream": body.as_ref().and_then(|value| value.get("stream")).and_then(Value::as_bool),
        "tools_count": body.as_ref().and_then(|value| value.get("tools")).and_then(Value::as_array).map_or(0, Vec::len),
    })
}

fn fingerprint(value: &str) -> String {
    if value.is_empty() {
        return "absent".to_string();
    }
    let bytes = value.as_bytes();
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(8)]);
    format!("len:{} head:{}", value.len(), head)
}

fn sse_body(id: &str, text: &str) -> String {
    format!(
        "event: response.created\n\
         data: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{id}\"}}}}\n\n\
         event: response.output_item.done\n\
         data: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"message\",\"role\":\"assistant\",\"id\":\"msg-001\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{text}\"}}]}}}}\n\n\
         event: response.completed\n\
         data: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{id}\",\"usage\":{{\"input_tokens\":1,\"output_tokens\":1,\"total_tokens\":2}}}}}}\n\n"
    )
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    chunked: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let headers = if chunked {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
    };
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn append_json_line(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{value}")?;
    Ok(())
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
