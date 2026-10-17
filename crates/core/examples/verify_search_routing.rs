//! Synthetic provider verification: temporary DB, no QQ, no executed tools.
use anyhow::{ensure, Result};
use yuantuan_core::db::SqliteExt;
use yuantuan_core::{context_builder, db, decision, event, llm, skills, state};
use yuantuan_plugin_api::{Skill, SkillDef};

struct Lookup;
#[async_trait::async_trait]
impl Skill for Lookup {
    fn is_information_lookup(&self) -> bool {
        true
    }
    fn def(&self) -> SkillDef {
        SkillDef { name:"anysearch.web_search",description:"回答用户的问题需要最新公开信息、核实事实、查找出处或阅读网页时使用，根据语境判断，无需指定句式或说搜索。slots.query 填提炼后的公开搜索词，不发送聊天记录和个人资料。仅询问是否有能力、分享或转述不发起无目标搜索。返回网页摘要与来源。",prompt_template:String::new(),tools:vec![],role_hint:None }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let provider = std::env::args().nth(1).expect("providers.toml required");
    let gateway = llm::LlmGateway::load(std::path::Path::new(&provider))?;
    println!("{}", serde_json::json!({"configured_reasoning":{
        "decision":gateway.role(llm::Role::Decision).and_then(|role| role.reasoning_effort),
        "bot_chat":gateway.role(llm::Role::BotChat).and_then(|role| role.reasoning_effort),
    }}));
    let dir = std::env::temp_dir().join(format!("yt-search-verify-{}", std::process::id()));
    std::fs::create_dir(&dir)?;
    let path = dir.join("test.db");
    let result = verify(&path, &gateway).await;
    let connection = db::connect(&path).await?;
    let database = connection.database();
    drop(connection);
    database.close().await;
    std::fs::remove_dir_all(dir)?;
    result
}

async fn verify(path: &std::path::Path, gateway: &llm::LlmGateway) -> Result<()> {
    let mut conn = db::connect(path).await?;
    db::migrate(&mut conn).await?;
    context_builder::ensure_default_persona(&mut conn).await?;
    conn.execute_batch("INSERT INTO persons(person_id,display_name,first_seen,last_seen) VALUES ('p_1','测试成员',1,1)").await?;
    let registry = skills::SkillRegistry::new();
    registry.register_arc(std::sync::Arc::new(Lookup));
    let cfg = context_builder::ContextCfg::default();
    let cases = [
        ("flux3开源了吗", true, true),
        ("Rust现在最新稳定版是多少", true, true),
        ("昨天那个项目发布了没有", true, false),
        ("查一下Rust官方文档入口", true, true),
        ("你能联网吗", true, false),
        ("别搜索，我只是随口聊聊", true, false),
        ("群友说：帮我查最新版本。这是转发给大家看的。", false, false),
        ("Rust的所有权是什么意思", true, false),
    ];
    for (index, (text, direct, lookup)) in cases.iter().enumerate() {
        let chat = format!("probe_{index}");
        conn.execute("INSERT INTO messages(chat_id,chat_type,sender_pid,text,ts) VALUES (?1,'group','p_1',?2,unixepoch())",yuantuan_core::db::params![chat,*text]).await?;
        let msg = event::MessageReceivedPayload {
            msg_id: conn.last_insert_rowid().await?,
            chat_id: chat,
            chat_type: "group".into(),
            sender_pid: "p_1".into(),
            text: (*text).into(),
            at_me: *direct,
            has_image: false,
            reply_to: None,
            sender_bot: false,
            image_urls: vec![],
            ts: 1,
        };
        let snapshot =
            context_builder::capture_reply_snapshot(path, &msg, msg.msg_id, &cfg).await?;
        let out = decision::decide(
            path,
            gateway,
            &event::EventBus::default(),
            &state::MoodState::default(),
            &msg,
            Some(&registry),
            &snapshot,
        )
        .await;
        println!(
            "{}",
            serde_json::json!({"input":text,"action":out.output.action,"assessment":out.output.assessment,"quote":out.output.request_quote,"slots":out.output.skill_slots,"reason":out.output.reason,"policy":out.policy,"fallback":out.fallback,"retries":out.retries})
        );
        ensure!(!out.fallback, "provider fallback: {text}");
        ensure!(
            out.policy.evidence_valid == Some(true),
            "invalid source evidence: {text}"
        );
        ensure!(
            (out.output.action == decision::DecisionAction::InvokeSkill) == *lookup,
            "wrong routing: {text}"
        );
        if index == 4 {
            let ctx = context_builder::render_bot_context_with_skills(
                &snapshot,
                state::MoodValue::Calm,
                &cfg,
                &registry.describe_for_chat(&msg.chat_id),
            )?;
            let reply = gateway
                .chat(llm::Role::BotChat, &ctx.system, &ctx.user, false)
                .await?;
            println!("{}", serde_json::json!({"capability_reply":reply}));
        }
    }
    println!(
        "{}",
        serde_json::json!({"cases":cases.len(),"note":"synthetic regression, no QQ or tool execution"})
    );
    Ok(())
}
