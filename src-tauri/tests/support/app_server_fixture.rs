use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::time::Duration;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--version") {
        if args.iter().any(|arg| arg == "--version-fails") {
            std::process::exit(2);
        }
        println!("codex-cli 0.147.0-fixture");
        return;
    }

    let log_path = args
        .windows(2)
        .find(|pair| pair[0] == "--fixture-log")
        .map(|pair| pair[1].clone())
        .expect("fixture log path");
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open fixture log");

    for line in io::stdin().lock().lines() {
        let line = line.expect("read JSONL request");
        writeln!(log, "{line}").expect("write fixture log");
        if line.contains("\"method\":\"initialized\"") {
            continue;
        }
        let id = request_id(&line);
        if line.contains("\"method\":\"initialize\"") {
            respond(id, r#"{"userAgent":"codex-cli/0.147.0-fixture","codexHome":"C:\\Users\\fixture\\.codex","platformFamily":"windows","platformOs":"windows"}"#);
        } else if line.contains("\"method\":\"thread/list\"") {
            let capability_probe = line.contains("\"limit\":1,") || line.contains("\"limit\":1}");
            if capability_probe && args.iter().any(|arg| arg == "--missing-thread-list") {
                respond_error(id, -32601, "method not found");
                continue;
            }
            if let Some(marker) = args
                .windows(2)
                .find(|pair| pair[0] == "--exit-first-list")
                .map(|pair| &pair[1])
                .filter(|marker| !capability_probe && !std::path::Path::new(marker).exists())
            {
                fs::write(marker, "exited\n").expect("record first exit");
                return;
            }
            if !capability_probe && args.iter().any(|arg| arg == "--always-exit-list") {
                return;
            }
            if !capability_probe && args.iter().any(|arg| arg == "--slow-list") {
                std::thread::sleep(Duration::from_secs(5));
            }
            let reconciled_state = args.windows(2).find_map(|pair| match pair[0].as_str() {
                "--lose-archive-response" => Some(("archived", pair[1].as_str())),
                "--lose-unarchive-response" => Some(("active", pair[1].as_str())),
                "--lose-delete-response" => Some(("deleted", pair[1].as_str())),
                _ => None,
            });
            if !capability_probe
                && reconciled_state
                    .is_some_and(|(_, marker)| std::path::Path::new(marker).exists())
            {
                let (state, _) = reconciled_state.expect("checked reconciled state");
                let requested_state = if line.contains("\"archived\":true") {
                    "archived"
                } else {
                    "active"
                };
                if state == requested_state {
                    respond(id, r#"{"data":[{"id":"thread-1","sessionId":"session-1","forkedFromId":null,"parentThreadId":null,"preview":"修复登录流程","ephemeral":false,"section":null,"sectionEnteredAt":null,"modelProvider":"history-provider","createdAt":1786900000,"updatedAt":1786900300,"recencyAt":1786900300,"status":{"type":"notLoaded"},"path":null,"cwd":"C:\\src\\demo","cliVersion":"0.147.0","source":"cli","threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":"登录修复","turns":[]}],"nextCursor":null,"backwardsCursor":null}"#);
                } else {
                    respond(id, r#"{"data":[],"nextCursor":null,"backwardsCursor":null}"#);
                }
                continue;
            }
            println!(
                "{}",
                r#"{"method":"thread/status/changed","params":{"threadId":"thread-1"}}"#
            );
            for index in 0..256 {
                eprintln!("fixture stderr {index}");
            }
            let response = if args.iter().any(|arg| arg == "--legacy-metadata") {
                r#"{"data":[{"id":"legacy-thread","preview":"旧版本会话","modelProvider":"legacy-provider","source":"cli"}],"nextCursor":null}"#
            } else if args.iter().any(|arg| arg == "--mixed-sources") {
                r#"{"data":[{"id":"thread-1","sessionId":"session-1","forkedFromId":null,"parentThreadId":null,"preview":"修复登录流程","ephemeral":false,"section":null,"sectionEnteredAt":null,"modelProvider":"history-provider","createdAt":1786900000,"updatedAt":1786900300,"recencyAt":1786900300,"status":{"type":"notLoaded"},"path":null,"cwd":"C:\\src\\demo","cliVersion":"0.147.0","source":"cli","threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":"登录修复","turns":[]},{"id":"thread-exec","sessionId":"session-exec","forkedFromId":null,"parentThreadId":null,"preview":"内部 exec","ephemeral":false,"section":null,"sectionEnteredAt":null,"modelProvider":"history-provider","createdAt":1786900001,"updatedAt":1786900301,"recencyAt":1786900301,"status":{"type":"notLoaded"},"path":null,"cwd":"C:\\src\\demo","cliVersion":"0.147.0","source":"exec","threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":"内部 exec","turns":[]},{"id":"thread-subagent","sessionId":"session-subagent","forkedFromId":null,"parentThreadId":"thread-1","preview":"子代理内部会话","ephemeral":false,"section":null,"sectionEnteredAt":null,"modelProvider":"history-provider","createdAt":1786900002,"updatedAt":1786900302,"recencyAt":1786900302,"status":{"type":"notLoaded"},"path":null,"cwd":"C:\\src\\demo","cliVersion":"0.147.0","source":"subAgent","threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":"子代理内部会话","turns":[]}],"nextCursor":"cursor-2","backwardsCursor":"cursor-back"}"#
            } else {
                r#"{"data":[{"id":"thread-1","sessionId":"session-1","forkedFromId":null,"parentThreadId":null,"preview":"修复登录流程","ephemeral":false,"section":null,"sectionEnteredAt":null,"modelProvider":"history-provider","createdAt":1786900000,"updatedAt":1786900300,"recencyAt":1786800300,"status":{"type":"notLoaded"},"path":null,"cwd":"C:\\src\\demo","cliVersion":"0.147.0","source":"cli","threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":"登录修复","turns":[]}],"nextCursor":"cursor-2","backwardsCursor":"cursor-back"}"#
            };
            respond(id, response);
        } else if line.contains("\"method\":\"thread/read\"") {
            if line.contains("gpteasy-capability-probe-invalid-session") {
                if args.iter().any(|arg| arg == "--missing-thread-read") {
                    respond_error(id, -32601, "method not found");
                } else {
                    respond_error(id, -32001, "session not found");
                }
                continue;
            }
            respond(id, r#"{"thread":{"id":"thread-1","sessionId":"session-1","forkedFromId":null,"parentThreadId":null,"preview":"修复登录流程","ephemeral":false,"section":null,"sectionEnteredAt":null,"modelProvider":"history-provider","createdAt":1786900000,"updatedAt":1786900300,"recencyAt":1786900300,"status":{"type":"notLoaded"},"path":null,"cwd":"C:\\src\\demo","cliVersion":"0.147.0","source":"cli","threadSource":null,"agentNickname":null,"agentRole":null,"gitInfo":null,"name":"登录修复","turns":[{"id":"turn-1","items":[{"type":"userMessage","id":"item-1","clientId":null,"content":[{"type":"text","text":"请修复登录","text_elements":[]}]},{"type":"commandExecution","id":"item-2","pluginId":null,"scriptPath":null,"command":"npm test","cwd":"C:\\src\\demo","processId":null,"source":"agent","status":"completed","commandActions":[],"aggregatedOutput":"all passed","exitCode":0,"durationMs":1200},{"type":"agentMessage","id":"item-3","text":"登录流程已修复。","phase":null,"memoryCitation":null}],"itemsView":{"type":"full"},"status":"completed","error":null,"startedAt":1786900010,"completedAt":1786900020,"durationMs":10000}]}}"#);
        } else if line.contains("\"method\":\"thread/archive\"") {
            if line.contains("gpteasy-capability-probe-invalid-session") {
                if args.iter().any(|arg| arg == "--missing-thread-archive") {
                    respond_error(id, -32601, "method not found");
                } else {
                    respond_error(id, -32001, "session not found");
                }
                continue;
            }
            if let Some(marker) = args
                .windows(2)
                .find(|pair| pair[0] == "--lose-archive-response")
                .map(|pair| &pair[1])
                .filter(|marker| !std::path::Path::new(marker).exists())
            {
                fs::write(marker, "archived\n").expect("record archived state");
                return;
            }
            if args
                .windows(2)
                .find(|pair| pair[0] == "--fail-archive")
                .is_some_and(|pair| line.contains(&format!("\"threadId\":\"{}\"", pair[1])))
            {
                respond_error(id, -32002, "archive failed");
            } else {
                respond(id, "{}");
            }
        } else if line.contains("\"method\":\"thread/unarchive\"") {
            if line.contains("gpteasy-capability-probe-invalid-session") {
                if args.iter().any(|arg| arg == "--missing-thread-unarchive") {
                    respond_error(id, -32601, "method not found");
                } else {
                    respond_error(id, -32001, "session not found");
                }
            } else if let Some(marker) = args
                .windows(2)
                .find(|pair| pair[0] == "--lose-unarchive-response")
                .map(|pair| &pair[1])
                .filter(|marker| !std::path::Path::new(marker).exists())
            {
                fs::write(marker, "active\n").expect("record active state");
                return;
            } else {
                respond(id, "{}");
            }
        } else if line.contains("\"method\":\"thread/delete\"") {
            if line.contains("gpteasy-capability-probe-invalid-session") {
                if args.iter().any(|arg| arg == "--missing-thread-delete") {
                    respond_error(id, -32601, "method not found");
                } else {
                    respond_error(id, -32001, "session not found");
                }
            } else if let Some(marker) = args
                .windows(2)
                .find(|pair| pair[0] == "--lose-delete-response")
                .map(|pair| &pair[1])
                .filter(|marker| !std::path::Path::new(marker).exists())
            {
                fs::write(marker, "deleted\n").expect("record deleted state");
                return;
            } else {
                respond(id, "{}");
            }
        } else {
            respond_error(id, -32601, "method not found");
        }
    }
    writeln!(log, "EOF").expect("record EOF");
}

fn request_id(line: &str) -> u64 {
    line.split("\"id\":")
        .nth(1)
        .and_then(|tail| tail.split(|character: char| !character.is_ascii_digit()).next())
        .and_then(|value| value.parse().ok())
        .expect("numeric request id")
}

fn respond(id: u64, result: &str) {
    println!(r#"{{"id":{id},"result":{result}}}"#);
    io::stdout().flush().expect("flush response");
}

fn respond_error(id: u64, code: i64, message: &str) {
    println!(r#"{{"id":{id},"error":{{"code":{code},"message":"{message}"}}}}"#);
    io::stdout().flush().expect("flush error response");
}
