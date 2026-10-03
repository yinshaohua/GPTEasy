//! Opt-in round trip through the installed Codex; uses no real credentials.
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdout, Command};

async fn reply(stdout: &mut BufReader<ChildStdout>, id: u64) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let mut line = String::new();
            assert!(stdout.read_line(&mut line).await.expect("read App Server") > 0);
            let message: Value = serde_json::from_str(&line).expect("App Server JSON");
            if message["id"] == id {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    })
    .await
    .expect("App Server reply timeout")
}

#[tokio::test]
#[ignore = "requires GPTEASY_CODEX_EXECUTABLE pointing to an installed native Codex executable"]
async fn installed_codex_lists_common_reasoning_choices_with_and_without_root_none() {
    let executable = std::env::var_os("GPTEASY_CODEX_EXECUTABLE")
        .expect("set GPTEASY_CODEX_EXECUTABLE to a native executable, not a shell wrapper");
    for root_effort in ["", "model_reasoning_effort = \"none\"\n"] {
        let home = tempfile::tempdir().expect("isolated Codex home");
        std::fs::write(
            home.path().join("catalog.json"),
            super::render(
                &[
                    "gpt-new-model".to_owned(),
                    "deepseek-new-model".to_owned(),
                    "vendor-model".to_owned(),
                ],
                "vendor-model",
            )
            .expect("GPTEasy catalog"),
        )
        .expect("write catalog");
        let config = format!(
            "model = \"vendor-model\"\nmodel_catalog_json = \"catalog.json\"\n{root_effort}model_provider = \"fixture\"\n\
             [model_providers.fixture]\nname = \"Fixture\"\nbase_url = \"http://127.0.0.1:9/v1\"\n\
             wire_api = \"responses\"\nrequires_openai_auth = true\nsupports_websockets = false\n"
        );
        std::fs::write(home.path().join("config.toml"), config).expect("write config");
        std::fs::write(
            home.path().join("auth.json"),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture-key"}"#,
        )
        .expect("write fixture credentials");
        let mut command = Command::new(&executable);
        command
            .arg("app-server")
            .current_dir(home.path())
            .env("CODEX_HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command.spawn().expect("start isolated App Server");
        let mut stdin = child.stdin.take().expect("stdin");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let initialize = json!({"id":1,"method":"initialize","params":{
            "clientInfo":{"name":"gpteasy_catalog_contract","version":"1.0"},
            "capabilities":{"experimentalApi":true}
        }});
        stdin
            .write_all(format!("{initialize}\n").as_bytes())
            .await
            .expect("initialize");
        reply(&mut stdout, 1).await;
        stdin.write_all(b"{\"method\":\"initialized\"}\n{\"id\":2,\"method\":\"model/list\",\"params\":{\"includeHidden\":true,\"limit\":100}}\n")
            .await.expect("model/list");
        let result = reply(&mut stdout, 2).await;
        child.kill().await.expect("stop isolated App Server");
        let models = result["data"].as_array().expect("models");
        for model_id in ["gpt-new-model", "deepseek-new-model", "vendor-model"] {
            let model = models
                .iter()
                .find(|model| model["model"] == model_id)
                .expect("discovered model");
            assert_eq!(model["defaultReasoningEffort"], "high");
            let efforts = model["supportedReasoningEfforts"]
                .as_array()
                .expect("choices")
                .iter()
                .map(|choice| choice["reasoningEffort"].as_str().expect("effort"))
                .collect::<Vec<_>>();
            assert_eq!(
                efforts,
                ["low", "medium", "high", "xhigh"],
                "root setting: {root_effort:?}"
            );
        }
    }
}
