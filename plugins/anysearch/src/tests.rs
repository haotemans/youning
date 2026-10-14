use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use std::sync::Mutex;

#[derive(Clone, Default)]
struct ServerState {
    requests: Arc<Mutex<Vec<Value>>>,
    fail_domains: Arc<Mutex<bool>>,
    authorization: Arc<Mutex<Vec<Option<String>>>>,
}
struct Server {
    origin: String,
    state: ServerState,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server() -> Server {
    async fn search(
        State(state): State<ServerState>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        assert_eq!(headers.get("x-anysearch-client").unwrap(), "skill/3.1.1");
        state.authorization.lock().unwrap().push(
            headers
                .get("authorization")
                .map(|h| h.to_str().unwrap().to_owned()),
        );
        state.requests.lock().unwrap().push(body.clone());
        match body["query"].as_str().unwrap() {
            "quota" => {
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"message":"private-test-key","data":{"api_key":"auto-key"}})),
                )
            }
            "business-error" => {
                return (
                    StatusCode::OK,
                    Json(json!({"code":42,"message":"private-test-key"})),
                )
            }
            "huge" => {
                return (
                    StatusCode::OK,
                    Json(json!({"code":0,"data":{"padding":"x".repeat(RESPONSE_LIMIT+1)}})),
                )
            }
            _ => {}
        }
        (
            StatusCode::OK,
            Json(json!({"code":0,"api_key":"auto-key","data":{"results":[
            {"title":"Rust","url":"https://www.rust-lang.org/","content":"Systems language","api_key":"should-not-return"},
            {"title":"Invalid","url":"javascript:alert(1)","content":"bad"}
        ],"api_key":"auto-key"}})),
        )
    }
    async fn domains(State(state): State<ServerState>) -> (StatusCode, Json<Value>) {
        state.requests.lock().unwrap().push(json!({"catalog":true}));
        if *state.fail_domains.lock().unwrap() {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"message":"unavailable"})),
            );
        }
        (
            StatusCode::OK,
            Json(
                json!({"code":0,"data":{"domains":[{"domain":"code","sub_domains":[{"sub_domain":"code.doc","description":"documentation","params":{"library":{"required":true,"description":"name"}}}]}]}}),
            ),
        )
    }
    async fn extract(Json(body): Json<Value>) -> Json<Value> {
        Json(
            json!({"code":0,"data":{"url":body["url"],"title":"Page","content":"文".repeat(5000),"api_key":"auto-key"}}),
        )
    }
    let state = ServerState::default();
    let app = Router::new()
        .route("/v1/search", post(search))
        .route("/v1/sub-domains", get(domains))
        .route("/v1/extract", post(extract))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        origin,
        state,
        task,
    }
}
fn ctx() -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "g1".into(),
        chat_type: "group".into(),
        sender_pid: "p1".into(),
        locale: None,
    }
}
struct FakeHost;
#[async_trait]
impl HostServices for FakeHost {
    async fn chat(&self, _: &ToolCtx, system: &str, _: &str) -> Result<String> {
        Ok(if system.contains("只分类") {
            r#"{"domain":"code"}"#
        } else {
            r#"{"tag":"code.doc","params":{"library":"rust"}}"#
        }
        .into())
    }
    async fn read_data(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn write_data(&self, _: &str, _: Value) -> Result<()> {
        Ok(())
    }
    fn log(&self, _: &str) {}
}

#[tokio::test]
async fn search_preserves_sources_and_excludes_envelope_credentials() {
    let s = server().await;
    let api = Client::new(&s.origin, "private-test-key", 5).unwrap();
    let data = api.search(&json!({"query":"Rust"}), 10).await.unwrap();
    assert_eq!(data["results"].as_array().unwrap().len(), 1);
    assert_eq!(data["results"][0]["url"], "https://www.rust-lang.org/");
    assert!(!data.to_string().contains("key"));
    assert!(render(&data).contains("来源：https://www.rust-lang.org/"));
}
#[tokio::test]
async fn vertical_search_discovers_and_fills_required_parameters() {
    let s = server().await;
    let api = Client::new(&s.origin, "", 5).unwrap();
    api.search(&json!({"query":"Rust","tag":"code.doc"}), 10)
        .await
        .unwrap();
    let requests = s.state.requests.lock().unwrap();
    assert_eq!(requests[0]["catalog"], true);
    assert_eq!(requests[1]["params"]["library"], "");
    assert_eq!(requests[1]["tag"], "code.doc");
}
#[tokio::test]
async fn errors_and_large_responses_are_bounded_and_redacted() {
    let s = server().await;
    let api = Client::new(&s.origin, "private-test-key", 5).unwrap();
    for query in ["quota", "business-error", "huge"] {
        let error = api
            .search(&json!({"query":query}), 10)
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private-test-key") && !error.contains("auto-key"));
    }
}
#[tokio::test]
async fn extraction_is_bounded_and_marked_untrusted() {
    let s = server().await;
    let api = Client::new(&s.origin, "", 5).unwrap();
    let data = api
        .extract(&json!({"url":"https://example.com/page"}))
        .await
        .unwrap();
    assert_eq!(data["content"].as_str().unwrap().chars().count(), 4000);
    assert_eq!(data["truncated"], true);
    assert_eq!(data["untrusted"], true);
    assert!(!data.to_string().contains("auto-key"));
    assert!(api.extract(&json!({"url":"file:///secret"})).await.is_err());
}
#[tokio::test]
async fn batch_has_explicit_per_query_failures_and_limits() {
    let s = server().await;
    let tool = ApiTool {
        api: Arc::new(Client::new(&s.origin, "", 5).unwrap()),
        op: Operation::Batch,
    };
    let out = tool
        .call(
            &ctx(),
            json!({"queries":[{"query":"Rust"},{"query":"quota"}]}),
        )
        .await
        .unwrap();
    assert_eq!(out.data["queries"][0]["ok"], true);
    assert_eq!(out.data["queries"][1]["ok"], false);
    assert!(tool.call(&ctx(), json!({"queries":[]})).await.is_err());
    assert!(tool.call(&ctx(), json!({"queries":[{}]})).await.is_err());
}
#[tokio::test]
async fn chat_skill_routes_and_returns_sources_without_private_context() {
    let s = server().await;
    let skill = SearchSkill {
        api: Arc::new(Client::new(&s.origin, "", 5).unwrap()),
        host: Arc::new(FakeHost),
    };
    let out = skill
        .invoke(
            &ctx(),
            json!({"query":"Rust","context":"private conversation","user_text":"private profile"}),
        )
        .await
        .unwrap();
    assert!(out.data["rendered"]
        .as_str()
        .unwrap()
        .contains("https://www.rust-lang.org/"));
    let requests = s.state.requests.lock().unwrap();
    assert!(!serde_json::to_string(&*requests)
        .unwrap()
        .contains("private"));
    assert_eq!(requests.len(), 2, "目录只应获取一次，随后直接搜索");
    assert_eq!(requests[0]["catalog"], true);
    assert_eq!(requests.last().unwrap()["params"]["library"], "rust");
}
#[test]
fn config_validates_without_disclosing_credentials() {
    let plugin = AnySearchPlugin;
    assert!(plugin.validate_config(&json!({"max_results":11})).is_err());
    assert!(plugin
        .validate_config(&json!({"api_key":"bad\r\nvalue"}))
        .is_err());
    assert!(plugin
        .validate_config(&json!({"api_key":"","max_results":5}))
        .is_ok());
    assert!(query(&json!({"query":""})).is_err());
    for url in [
        "file:///tmp/test",
        "https://user:secret@example.com",
        "https://example.com/?key=secret",
        "https://example.com/#v1",
        "bad",
    ] {
        assert!(plugin
            .validate_config(&json!({"api_base_url":url}))
            .is_err());
    }
}

#[tokio::test]
async fn configured_endpoint_and_key_are_used_by_plugin() {
    let s = server().await;
    let caps = plugin()
        .start(
            PluginContext {
                host: Arc::new(FakeHost),
            },
            json!({"api_base_url":format!("{}/v1/",s.origin),"api_key":"configured-key"}),
        )
        .await
        .unwrap();
    caps.skills[0]
        .invoke(&ctx(), json!({"query":"Rust","explicit_command":true}))
        .await
        .unwrap();
    assert_eq!(
        *s.state.authorization.lock().unwrap(),
        vec![Some("Bearer configured-key".into())]
    );
    assert_eq!(s.state.requests.lock().unwrap().len(), 1);
}

/// Explicit live verification only: sends a public query with no API key.
#[tokio::test]
#[ignore]
async fn live_anonymous_search() {
    let api = Client::new("https://api.anysearch.com", "", 2).unwrap();
    let result = api
        .search(
            &json!({"query":"Rust programming language official website"}),
            2,
        )
        .await
        .unwrap();
    assert!(!result["results"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn domain_outage_falls_back_to_general_search() {
    let s = server().await;
    *s.state.fail_domains.lock().unwrap() = true;
    let skill = SearchSkill {
        api: Arc::new(Client::new(&s.origin, "", 5).unwrap()),
        host: Arc::new(FakeHost),
    };
    let out = skill
        .invoke(&ctx(), json!({"query":"Rust","domain":"code"}))
        .await
        .unwrap();
    assert!(out.data["rendered"]
        .as_str()
        .unwrap()
        .contains("领域目录暂不可用"));
    assert_eq!(out.data["results"].as_array().unwrap().len(), 1);
    assert!(s
        .state
        .requests
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .get("tag")
        .is_none());
}
#[tokio::test]
async fn explicit_command_skips_model_planning_and_domain_calls() {
    let s = server().await;
    *s.state.fail_domains.lock().unwrap() = true;
    let skill = SearchSkill {
        api: Arc::new(Client::new(&s.origin, "", 5).unwrap()),
        host: Arc::new(FakeHost),
    };
    skill
        .invoke(&ctx(), json!({"query":"Rust","explicit_command":true}))
        .await
        .unwrap();
    let requests = s.state.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["query"], "Rust");
}
