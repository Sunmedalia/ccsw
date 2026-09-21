//! Test-only executable; excluded from ordinary builds and release packages.
use serde_json::{Value, json};
use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};
fn emit(value: Value) {
    // Exercise Windows CRLF responses, even when running the fixture on Unix.
    print!("{}\r\n", value);
    io::stdout().flush().unwrap();
}
fn main() {
    if env::args().nth(1).as_deref() == Some("--sleep") {
        if let Some(path) = env::var_os("CCSW_HELPER_PID") {
            fs::write(path, std::process::id().to_string()).unwrap();
        }
        std::thread::sleep(Duration::from_secs(120));
        return;
    }
    if let Some(path) = env::var_os("CCSW_HELPER_RECORD") {
        fs::write(path, serde_json::to_vec(&json!({"args":env::args().skip(1).collect::<Vec<_>>(),"cwd":env::current_dir().unwrap(),"home":env::var_os("CODEX_HOME"),"literal":env::var("CCSW_HELPER_LITERAL").ok(),"has_api_key":env::var_os("OPENAI_API_KEY").is_some()})).unwrap()).unwrap();
    }
    if env::var_os("CCSW_HELPER_TREE").is_some() {
        let mut child = Command::new(env::current_exe().unwrap())
            .arg("--sleep")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        // The parent intentionally waits; Windows must terminate both members.
        child.wait().unwrap();
        return;
    }
    if env::args().nth(1).as_deref() == Some("--version") {
        println!("Claude Code 2.1.242");
        return;
    }
    if env::args().nth(1).as_deref() != Some("app-server") {
        return;
    }
    assert_eq!(
        env::args().skip(1).collect::<Vec<_>>(),
        ["app-server", "--stdio"]
    );
    let home = PathBuf::from(env::var_os("CODEX_HOME").unwrap());
    let fixture = PathBuf::from(env::var_os("CCSW_MOCK_AUTH").unwrap());
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(home.join("config.toml")).unwrap()).unwrap();
    assert_eq!(config["cli_auth_credentials_store"].as_str(), Some("file"));
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let Some(id) = request.get("id") else {
            continue;
        };
        let mut result = json!({});
        match request["method"].as_str().unwrap() {
            "account/login/start" => {
                fs::copy(&fixture, home.join("auth.json")).unwrap();
                emit(
                    json!({"method":"account/login/completed","params":{"loginId":"test-login","success":true}}),
                );
                result = json!({"type":"chatgptDeviceCode","loginId":"test-login","verificationUrl":"https://auth.openai.com/codex/device","userCode":"TEST-CODE"});
            }
            "account/read" => {
                let mut auth: Value =
                    serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
                auth["tokens"]["refresh_token"] = "refreshed-in-fixture".into();
                fs::write(home.join("auth.json"), serde_json::to_vec(&auth).unwrap()).unwrap();
                result = json!({"account":{"type":"chatgpt","email":"a@example.test","planType":"plus"}});
            }
            "account/rateLimits/read" => {
                if fixture.with_extension("fail").exists() {
                    emit(
                        json!({"id":id,"error":{"code":-1,"message":"SECRET_SHOULD_NEVER_BE_LOGGED"}}),
                    );
                    continue;
                }
                result = json!({"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300,"resetsAt":2000000000},"secondary":null},"rateLimitsByLimitId":null});
            }
            _ => {}
        }
        emit(json!({"id":id,"result":result}));
    }
}
