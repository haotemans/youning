//! Skills 系统（Q-S01 裁决：Rust trait，非 Markdown 文件）。
//!
//! Skill = 预定义提示词模板 + 可选 Tool 联动，对标 Anthropic Claude 的 SKILL.md。
//! 与 Tool 的差异：
//! - Tool 是执行型能力（发图、备份、写文件），返回 ToolOutput
//! - Skill 是提示词级别复用（会议纪要、翻译、总结），返回的还是 ToolOutput，但内容经 chat LLM 加工
//!
//! 生命周期：插件 crates 在 register() 里同时返回 Vec<Arc<dyn Tool>> 和 Vec<Arc<dyn Skill>>；
//!           main 装配时把 Skill 注入 SkillRegistry；Decision 看到 skill 描述列表后
//!           可以在 JSON 里返回 action=invoke_skill 主动触发（Q-S02 裁决）。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub use yuantuan_plugin_api::{
    arc_skill, build_slots, render_skill_catalog, render_template, validate_slots, Skill, SkillDef,
    SkillImpl,
};

/// Skill 注册表（类似 tools::Registry）
#[derive(Default, Clone)]
pub struct SkillRegistry {
    inner: Arc<RwLock<HashMap<&'static str, Arc<dyn Skill>>>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_arc(&self, skill: Arc<dyn Skill>) {
        let name = skill.def().name;
        self.inner
            .write()
            .expect("skills registry poisoned")
            .insert(name, skill);
    }

    pub fn register_batch(&self, items: Vec<Arc<dyn Skill>>) -> anyhow::Result<()> {
        let mut registry = self.inner.write().unwrap();
        let mut names = std::collections::HashSet::new();
        let mut commands: std::collections::HashSet<_> = registry
            .values()
            .flat_map(|s| s.commands().iter().copied())
            .collect();
        for tool in &items {
            let name = tool.def().name;
            if registry.contains_key(name) || !names.insert(name) {
                anyhow::bail!("能力名称冲突：{name}");
            }
            for command in tool.commands() {
                if !command.starts_with('/')
                    || command.len() <= 1
                    || command.chars().any(char::is_whitespace)
                    || ["/image", "/画"].contains(command)
                    || !commands.insert(*command)
                {
                    anyhow::bail!("插件命令无效或冲突：{command}");
                }
            }
        }
        for tool in items {
            registry.insert(tool.def().name, tool);
        }
        Ok(())
    }

    pub fn command(&self, text: &str) -> anyhow::Result<Option<(Arc<dyn Skill>, String)>> {
        let text = text.trim_start();
        let mut found = None;
        for skill in self.inner.read().unwrap().values() {
            for command in skill.commands() {
                if let Some(body) = text
                    .strip_prefix(command)
                    .filter(|s| s.is_empty() || s.starts_with(char::is_whitespace))
                {
                    if found.is_some() {
                        anyhow::bail!("插件命令存在冲突");
                    }
                    found = Some((skill.clone(), body.trim().to_string()));
                }
            }
        }
        Ok(found)
    }

    pub fn unregister(&self, names: &[&str]) {
        let mut registry = self.inner.write().unwrap();
        for name in names {
            registry.remove(*name);
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Skill>> {
        self.inner
            .read()
            .expect("skills registry poisoned")
            .get(name)
            .cloned()
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.inner
            .read()
            .expect("skills registry poisoned")
            .keys()
            .copied()
            .collect()
    }

    /// 列出每个 skill 的 (name, description)，供 Decision 系统提示注入
    pub fn describe_for_decision(&self) -> Vec<(String, String)> {
        self.inner
            .read()
            .expect("skills registry poisoned")
            .values()
            .map(|s| {
                let d = s.def();
                (d.name.to_string(), d.description.to_string())
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("skills registry poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use yuantuan_plugin_api::ToolCtx;

    struct CommandSkill(&'static str);
    #[async_trait::async_trait]
    impl Skill for CommandSkill {
        fn def(&self) -> SkillDef {
            SkillDef {
                name: self.0,
                description: "test",
                prompt_template: "".into(),
                tools: vec![],
                role_hint: None,
            }
        }
        fn commands(&self) -> &'static [&'static str] {
            &["/搜索", "/search"]
        }
    }
    #[test]
    fn commands_require_explicit_prefix_and_unique_registration() {
        let registry = SkillRegistry::new();
        registry
            .register_batch(vec![Arc::new(CommandSkill("first"))])
            .unwrap();
        assert_eq!(registry.command("/搜索 Rust").unwrap().unwrap().1, "Rust");
        assert_eq!(
            registry.command("/search\nRust").unwrap().unwrap().1,
            "Rust"
        );
        assert!(registry.command("/searching Rust").unwrap().is_none());
        assert!(registry.command("用户说 /搜索 Rust").unwrap().is_none());
        assert!(registry
            .register_batch(vec![Arc::new(CommandSkill("second"))])
            .is_err());
        assert!(registry.get("second").is_none());
    }

    fn make_skill(name: &'static str, desc: &'static str) -> Arc<dyn Skill> {
        arc_skill(SkillDef {
            name,
            description: desc,
            prompt_template: "处理：{user_text}".into(),
            tools: vec![],
            role_hint: None,
        })
        .unwrap()
    }

    #[test]
    fn template_replaces_slots() {
        let s = render_template(
            "你好 {name}，{msg}",
            &json!({ "name": "小明", "msg": "天气不错" }),
        );
        assert_eq!(s, "你好 小明，天气不错");
    }

    #[test]
    fn template_keeps_missing_slot() {
        let s = render_template("你好 {name}", &json!({}));
        assert_eq!(s, "你好 {name}");
    }

    #[test]
    fn registry_insert_and_get() {
        let reg = SkillRegistry::new();
        reg.register_arc(make_skill("meeting_notes", "整理会议纪要"));
        assert!(reg.get("meeting_notes").is_some());
        assert!(reg.get("not_exist").is_none());
        assert_eq!(reg.names().len(), 1);
    }

    #[test]
    fn catalog_renders_list() {
        let skills = vec![
            ("meeting_notes".to_string(), "整理会议纪要".to_string()),
            ("translate".to_string(), "翻译中文到英文".to_string()),
        ];
        let s = render_skill_catalog(&skills);
        assert!(s.contains("meeting_notes"));
        assert!(s.contains("整理会议纪要"));
    }

    #[test]
    fn catalog_empty_when_no_skills() {
        assert_eq!(render_skill_catalog(&[]), "");
    }

    #[tokio::test]
    async fn default_invoke_renders_template() {
        let skill = make_skill("test", "测试");
        let ctx = ToolCtx {
            task_id: None,
            chat_id: "c1".into(),
            chat_type: "group".into(),
            sender_pid: "p1".into(),
            locale: None,
        };
        let out = skill
            .invoke(&ctx, json!({ "user_text": "帮我整理" }))
            .await
            .unwrap();
        assert_eq!(out.data["skill"], "test");
        assert!(out.data["rendered"].as_str().unwrap().contains("帮我整理"));
    }

    #[test]
    fn arc_skill_rejects_empty_name() {
        let r = arc_skill(SkillDef {
            name: "",
            description: "x",
            prompt_template: "".into(),
            tools: vec![],
            role_hint: None,
        });
        assert!(r.is_err());
    }

    #[test]
    fn build_slots_helper() {
        let v = build_slots("hi", Some("ctx"));
        assert_eq!(v["user_text"], "hi");
        assert_eq!(v["context"], "ctx");
    }

    #[test]
    fn validate_slots_accepts_anything_v1() {
        let def = SkillDef {
            name: "x",
            description: "x",
            prompt_template: "".into(),
            tools: vec![],
            role_hint: None,
        };
        assert!(validate_slots(&def, &json!({})).is_ok());
    }
}
