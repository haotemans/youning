use super::*;
use std::sync::Mutex;
#[derive(Default)]
struct Host {
    actions: Mutex<Vec<ModerationAction>>,
    reject: bool,
}
#[async_trait]
impl HostServices for Host {
    async fn chat(&self, _: &ToolCtx, _: &str, _: &str) -> Result<String> {
        bail!("no models")
    }
    async fn read_data(&self, _: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn write_data(&self, _: &str, _: Value) -> Result<()> {
        Ok(())
    }
    fn log(&self, _: &str) {}
    async fn moderate(&self, _: &ToolCtx, action: ModerationAction) -> Result<()> {
        if self.reject {
            bail!("权限不足");
        }
        self.actions.lock().unwrap().push(action);
        Ok(())
    }
}
fn ctx() -> ToolCtx {
    ToolCtx {
        task_id: None,
        chat_id: "42".into(),
        chat_type: "group".into(),
        sender_pid: "p_123".into(),
        locale: None,
    }
}
#[tokio::test]
async fn commands_support_numbers_mentions_replies_and_unban_without_models() {
    let host = Arc::new(Host::default());
    let caps = plugin()
        .start(PluginContext { host: host.clone() }, json!({}))
        .await
        .unwrap();
    caps.skills[0]
        .invoke(
            &ctx(),
            json!({"explicit_command":true,"user_text":"","reply_to":-321}),
        )
        .await
        .unwrap();
    caps.skills[1]
        .invoke(
            &ctx(),
            json!({"explicit_command":true,"user_text":"456 10分钟"}),
        )
        .await
        .unwrap();
    caps.skills[1]
        .invoke(
            &ctx(),
            json!({"explicit_command":true,"user_text":"5","mentioned_qqs":[789]}),
        )
        .await
        .unwrap();
    caps.skills[2]
        .invoke(
            &ctx(),
            json!({"explicit_command":true,"user_text":"","mentioned_qqs":[456]}),
        )
        .await
        .unwrap();
    caps.skills[0]
        .invoke(&ctx(), json!({"reply_to":321}))
        .await
        .unwrap();
    assert_eq!(
        *host.actions.lock().unwrap(),
        vec![
            ModerationAction::Recall { message_id: -321 },
            ModerationAction::Ban {
                user_id: 456,
                duration_secs: 600
            },
            ModerationAction::Ban {
                user_id: 789,
                duration_secs: 300
            },
            ModerationAction::Ban {
                user_id: 456,
                duration_secs: 0
            },
            ModerationAction::Recall { message_id: 321 }
        ]
    );
}
#[tokio::test]
async fn rejects_unknown_operator_private_chat_bad_duration_and_ambiguous_target() {
    let host = Arc::new(Host::default());
    let caps = plugin()
        .start(
            PluginContext { host: host.clone() },
            json!({"operator_qqs":"123","max_ban_minutes":10}),
        )
        .await
        .unwrap();
    let mut other = ctx();
    other.sender_pid = "p_999".into();
    assert!(caps.tools[1]
        .call(&other, json!({"user_id":456,"duration_minutes":1}))
        .await
        .is_err());
    other = ctx();
    other.chat_type = "private".into();
    assert!(caps.tools[1]
        .call(&other, json!({"user_id":456,"duration_minutes":1}))
        .await
        .is_err());
    for minutes in [json!(0), json!(-1), json!(11), json!(1.5), json!("10")] {
        assert!(caps.tools[1]
            .call(&ctx(), json!({"user_id":456,"duration_minutes":minutes}))
            .await
            .is_err());
    }
    assert!(caps.skills[1]
        .invoke(
            &ctx(),
            json!({"explicit_command":true,"user_text":"10","mentioned_qqs":[456,789]})
        )
        .await
        .is_err());
    assert!(host.actions.lock().unwrap().is_empty());
}
#[tokio::test]
async fn never_reports_success_when_adapter_rejects() {
    let caps = plugin()
        .start(
            PluginContext {
                host: Arc::new(Host {
                    reject: true,
                    ..Default::default()
                }),
            },
            json!({}),
        )
        .await
        .unwrap();
    assert!(caps.tools[0]
        .call(&ctx(), json!({"message_id":321}))
        .await
        .unwrap_err()
        .to_string()
        .contains("权限不足"));
}
#[test]
fn validates_config() {
    assert!(plugin()
        .validate_config(&json!({"operator_qqs":"123，456","max_ban_minutes":43200}))
        .is_ok());
    for config in [
        json!({"operator_qqs":"abc"}),
        json!({"max_ban_minutes":0}),
        json!({"max_ban_minutes":43201}),
    ] {
        assert!(plugin().validate_config(&config).is_err());
    }
}
