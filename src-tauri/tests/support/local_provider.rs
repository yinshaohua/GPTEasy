use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};

pub struct LocalProvider {
    base_url: String,
    streaming: Option<Receiver<()>>,
    worker: JoinHandle<Result<(), &'static str>>,
}

impl LocalProvider {
    pub fn compatible(expected_api_key: String, model: &'static str) -> Self {
        Self::start(expected_api_key, ResponseMode::Compatible { model })
    }

    pub fn authentication_failure(expected_api_key: String) -> Self {
        Self::start(expected_api_key, ResponseMode::AuthenticationFailure)
    }

    pub fn cancellable(expected_api_key: String, model: &'static str) -> Self {
        Self::start(expected_api_key, ResponseMode::Cancellable { model })
    }

    fn start(expected_api_key: String, mode: ResponseMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local provider");
        let address = listener.local_addr().expect("read local provider address");
        let (streaming_sender, streaming) = if matches!(mode, ResponseMode::Cancellable { .. }) {
            let (sender, receiver) = mpsc::channel();
            (Some(sender), Some(receiver))
        } else {
            (None, None)
        };
        let worker =
            thread::spawn(move || serve(listener, &expected_api_key, mode, streaming_sender));
        Self {
            base_url: format!("http://{address}"),
            streaming,
            worker,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn wait_until_streaming(&mut self) {
        self.streaming
            .take()
            .expect("provider was not configured for cancellation")
            .recv_timeout(Duration::from_secs(2))
            .expect("provider did not enter the streaming response");
    }

    pub fn finish(self) {
        self.worker
            .join()
            .expect("local provider worker must not panic")
            .expect("local provider must observe the expected protocol");
    }
}

#[derive(Clone, Copy)]
enum ResponseMode {
    Compatible { model: &'static str },
    Cancellable { model: &'static str },
    AuthenticationFailure,
}

fn serve(
    listener: TcpListener,
    expected_api_key: &str,
    mode: ResponseMode,
    streaming: Option<Sender<()>>,
) -> Result<(), &'static str> {
    let request_count = match mode {
        ResponseMode::Compatible { .. } => 3,
        ResponseMode::Cancellable { .. } => 2,
        ResponseMode::AuthenticationFailure => 1,
    };
    for index in 0..request_count {
        let (mut stream, _) = listener
            .accept()
            .map_err(|_| "local provider did not accept a request")?;
        let request = read_request(&mut stream)?;
        if !has_expected_authorization(&request, expected_api_key) {
            return Err("validation did not use the expected API key");
        }
        match mode {
            ResponseMode::AuthenticationFailure => write_json(
                &mut stream,
                "401 Unauthorized",
                r#"{"error":{"message":"authentication failed"}}"#,
            )?,
            ResponseMode::Compatible { model } | ResponseMode::Cancellable { model }
                if index == 0 =>
            {
                write_json(
                    &mut stream,
                    "200 OK",
                    &format!(r#"{{"object":"list","data":[{{"id":"{model}"}}]}}"#),
                )?
            }
            ResponseMode::Compatible { .. } => {
                let payload = serde_json::from_slice(request_body(&request))
                    .map_err(|_| "validation request body was not JSON")?;
                write_validation_sse(&mut stream, &payload)?;
            }
            ResponseMode::Cancellable { .. } => {
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
                    )
                    .and_then(|_| stream.flush())
                    .map_err(|_| "could not open cancellable SSE response")?;
                streaming
                    .as_ref()
                    .ok_or("cancellable provider signal was missing")?
                    .send(())
                    .map_err(|_| "cancellable provider signal was not observed")?;
                thread::sleep(Duration::from_millis(500));
            }
        }
    }
    Ok(())
}

fn has_expected_authorization(request: &[u8], expected_api_key: &str) -> bool {
    let headers = request
        .split(|byte| *byte == b'\n')
        .take_while(|line| !matches!(*line, b"\r" | b""))
        .map(|line| String::from_utf8_lossy(line).trim().to_owned())
        .collect::<Vec<_>>();
    let expected = format!("Bearer {expected_api_key}");
    headers.iter().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == expected
        })
    })
}

fn read_request(stream: &mut TcpStream) -> Result<Vec<u8>, &'static str> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| "could not set local provider timeout")?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let mut expected_length = None;
    loop {
        let count = stream
            .read(&mut chunk)
            .map_err(|_| "could not read local provider request")?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if expected_length.is_none()
            && let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or_default();
            expected_length = Some(header_end + 4 + content_length);
        }
        if expected_length.is_some_and(|length| bytes.len() >= length) {
            break;
        }
    }
    Ok(bytes)
}

fn request_body(request: &[u8]) -> &[u8] {
    find_bytes(request, b"\r\n\r\n")
        .map(|index| &request[index + 4..])
        .unwrap_or_default()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn write_json(stream: &mut TcpStream, status: &str, body: &str) -> Result<(), &'static str> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|_| "could not write local provider JSON")?;
    stream
        .flush()
        .map_err(|_| "could not flush local provider JSON")
}

fn write_validation_sse(stream: &mut TcpStream, payload: &Value) -> Result<(), &'static str> {
    let has_tool_output = payload["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "function_call_output")
    });
    let events = if has_tool_output {
        let output = payload["input"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["type"] == "function_call_output")
            })
            .and_then(|item| item["output"].as_str())
            .and_then(|output| serde_json::from_str::<Value>(output).ok())
            .ok_or("tool output was missing")?;
        let nonce = output["nonce"]
            .as_str()
            .ok_or("tool output nonce was missing")?;
        vec![
            sse_event(
                "response.output_text.delta",
                json!({"delta": format!("validated {nonce}")}),
            ),
            sse_event(
                "response.completed",
                json!({"response":{"id":"response-final"}}),
            ),
        ]
    } else {
        let requested_nonce = payload["input"][0]["content"][0]["text"]
            .as_str()
            .and_then(extract_backtick_value)
            .ok_or("requested nonce was missing")?;
        let arguments = json!({"nonce": requested_nonce}).to_string();
        vec![
            sse_event(
                "response.function_call_arguments.delta",
                json!({"item_id":"call-item","delta":arguments}),
            ),
            sse_event(
                "response.output_item.done",
                json!({"item":{"type":"function_call","id":"call-item","call_id":"call-001","name":"gpteasy_probe","arguments":arguments}}),
            ),
            sse_event(
                "response.completed",
                json!({"response":{"id":"response-tool"}}),
            ),
        ]
    };
    let body = events.join("");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream; charset=utf-8\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n"
    )
    .map_err(|_| "could not write SSE headers")?;
    for fragment in body.as_bytes().chunks(7) {
        write!(stream, "{:X}\r\n", fragment.len())
            .map_err(|_| "could not write SSE chunk length")?;
        stream
            .write_all(fragment)
            .and_then(|_| stream.write_all(b"\r\n"))
            .map_err(|_| "could not write SSE chunk")?;
    }
    stream
        .write_all(b"0\r\n\r\n")
        .and_then(|_| stream.flush())
        .map_err(|_| "could not finish SSE response")
}

fn sse_event(kind: &str, mut payload: Value) -> String {
    payload
        .as_object_mut()
        .expect("SSE payload must be an object")
        .insert("type".to_owned(), Value::String(kind.to_owned()));
    format!("event: {kind}\ndata: {payload}\n\n")
}

fn extract_backtick_value(text: &str) -> Option<&str> {
    let start = text.find('`')? + 1;
    let end = text[start..].find('`')? + start;
    Some(&text[start..end])
}
