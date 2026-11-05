use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum MembershipAction {
    List {
        kind: String,
    },
    Inspect {
        id: i64,
    },
    Decide {
        id: i64,
        approve: bool,
        reason: String,
        remark: String,
    },
    AddFriend {
        user_id: u64,
        remark: String,
    },
    // These are not exposed as model tools. The host supplies the applied plugin policy.
    AutoPending,
    AutoInspect {
        id: i64,
    },
    AutoDecide {
        id: i64,
        approve: bool,
        reason: String,
        confidence: f64,
        rule_quote: String,
    },
    Defer {
        id: i64,
        reason: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionRule {
    pub rules: String,
    #[serde(default = "default_min_level")]
    pub min_qq_level: u16,
}

fn default_min_level() -> u16 {
    16
}

pub fn admission_rules(config: &Value) -> Result<BTreeMap<String, AdmissionRule>> {
    let text = config
        .get("group_rules")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    ensure!(text.len() <= 64_000, "群审核规则过长");
    let rules: BTreeMap<String, AdmissionRule> =
        serde_json::from_str(text).context("群审核规则须为按群号配置的 JSON 对象")?;
    ensure!(rules.len() <= 64, "最多配置64个群的审核规则");
    for (group, rule) in &rules {
        ensure!(
            group.parse::<u64>().ok().is_some_and(|id| id > 0),
            "群号无效"
        );
        ensure!(
            !rule.rules.trim().is_empty() && rule.rules.chars().count() <= 2000,
            "群规则须为1–2000字"
        );
        ensure!(
            (16..=256).contains(&rule.min_qq_level),
            "QQ等级门槛须为16–256，至少一个太阳"
        );
    }
    Ok(rules)
}
