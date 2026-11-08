//! 参与判断：模型解释语境；Runtime 验证来源、控制打扰度和选择回复方式。
use crate::decision::{DecisionAction, DecisionOutput, ReplyLen};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
mod activity;
pub use activity::{Activity, Heat};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    Bot,
    Other,
    Group,
    Unclear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Question,
    Request,
    FollowUp,
    Sharing,
    Acknowledgment,
    Banter,
    Stop,
    Correction,
    Other,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Continuity {
    NewTopic,
    Continuing,
    Closing,
    Unclear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    Medium,
    High,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Novelty {
    New,
    Repeated,
    Unclear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evidence {
    Sufficient,
    Missing,
    Conflicting,
    NotNeeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub audience: Audience,
    pub intent: Intent,
    pub continuity: Continuity,
    pub confidence: Level,
    pub benefit: Level,
    pub novelty: Novelty,
    pub evidence: Evidence,
    pub evidence_msg_ids: Vec<i64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyMode {
    #[default]
    Answer,
    Clarify,
    StateUncertainty,
    Social,
}
impl ReplyMode {
    pub fn instruction(self) -> &'static str {
        match self {
            Self::Answer => "回复方式：直接回答当前问题，回答完就停；参与判断不是新的事实来源。",
            Self::Clarify => "回复方式：只澄清解决当前问题必需的一项缺失信息，不编造前提，不为了延长聊天追问。",
            Self::StateUncertainty => "回复方式：明确说明现有资料无法确认或有冲突，不猜肯定/否定的经历和执行结果；能确认的部分可简短说明。",
            Self::Social => "回复方式：自然接上当前公开话题，沿用你现有的人设，用一两句具体、有回应感的话点评、共情、接梗或顺着聊。不要写客服开场、知识总结、待办列表或句句追问要不要帮忙；不复读群友或自己，不凭空编个人经历、图片内容或共同往事。接话不调用工具、不安排任务、不声称已经执行。",
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct PolicyTrace {
    pub version: u8,
    pub suggested_action: String,
    pub evidence_valid: Option<bool>,
    pub cooldown_secs: Option<i64>,
    pub notes: Vec<String>,
    pub activity: Option<Activity>,
    pub participation: Option<String>,
}

pub struct PolicyInput<'a> {
    pub is_group: bool,
    pub direct: bool,
    /// A private channel or unambiguous protocol @, never inferred from prose.
    pub protocol_direct: bool,
    pub information_lookup: bool,
    pub anchor_id: i64,
    pub visible_ids: &'a BTreeSet<i64>,
    pub linked_reply_id: Option<i64>,
    pub reply_age: Option<i64>,
    pub bot_bubbles_5min: i64,
    pub human_messages_30s: i64,
    pub activity: Option<&'a Activity>,
}

fn suppress(out: &mut DecisionOutput, trace: &mut PolicyTrace, reason: &str) {
    out.action = DecisionAction::Ignore;
    out.mention = false;
    trace.notes.push(reason.into());
}

/// Validate source evidence and apply participation policy after semantic routing.
pub fn apply(out: &mut DecisionOutput, input: &PolicyInput<'_>, trace: &mut PolicyTrace) {
    let social = out.reply_mode == ReplyMode::Social;
    if social {
        out.reply_len = ReplyLen::Short;
        out.mention = false;
        out.task_control = None;
        out.task_plan = None;
        out.task_goal = None;
        out.skill_name = None;
        out.skill_slots = None;
        out.request_quote = None;
        if !matches!(
            out.action,
            DecisionAction::Reply | DecisionAction::SendMeme | DecisionAction::Ignore
        ) {
            suppress(out, trace, "自然接话不授权执行技能或派发任务");
            return;
        }
        if out.assessment.is_none() {
            suppress(out, trace, "接话缺少可验证的语境判断");
            return;
        }
    }
    let Some(mut a) = out.assessment.clone() else {
        return;
    };
    let valid = !a.evidence_msg_ids.is_empty()
        && a.evidence_msg_ids.len() <= 6
        && a.evidence_msg_ids.contains(&input.anchor_id)
        && a.evidence_msg_ids
            .iter()
            .all(|id| input.visible_ids.contains(id));
    trace.evidence_valid = Some(valid);
    if input.protocol_direct && a.audience == Audience::Other {
        a.audience = Audience::Bot;
        out.assessment = Some(a.clone());
        trace
            .notes
            .push("协议确认消息明确指向云团，引用作者或云团QQ编号不改变接收对象".into());
        // Correct only a demonstrably contradictory target classification.
        // An ordinary ignore, low confidence, closing or stop is still silent.
        if out.action == DecisionAction::Ignore
            && valid
            && a.confidence == Level::High
            && a.continuity != Continuity::Closing
            && matches!(
                a.intent,
                Intent::Request | Intent::Question | Intent::FollowUp | Intent::Correction
            )
        {
            out.action = DecisionAction::Reply;
            out.reply_mode = ReplyMode::Answer;
            out.reply_len = ReplyLen::Short;
            out.mention = false;
            out.task_control = None;
            out.task_plan = None;
            out.task_goal = None;
            out.skill_name = None;
            out.skill_slots = None;
            out.request_quote = None;
            out.reason = "当前用户明确向云团提出请求，按其要求回应".into();
            trace
                .notes
                .push("修正other误判为普通文字回应，不补调Decision或授权执行".into());
        }
    }
    if out.action == DecisionAction::Ignore {
        return;
    }
    if !valid {
        suppress(out, trace, "参与判断的消息依据不在本次可见输入中");
        return;
    }
    if a.intent == Intent::Stop || (input.is_group && a.continuity == Continuity::Closing) {
        suppress(out, trace, "对方在结束对话，不再延长交谈");
        return;
    }
    if input.is_group && a.audience == Audience::Other {
        suppress(out, trace, "当前发言面向其他成员");
        return;
    }
    let linked_followup = input
        .linked_reply_id
        .is_some_and(|id| a.evidence_msg_ids.contains(&id))
        && a.audience == Audience::Bot
        && matches!(
            a.intent,
            Intent::FollowUp | Intent::Acknowledgment | Intent::Question | Intent::Request
        )
        && a.continuity == Continuity::Continuing
        && a.confidence == Level::High;
    if matches!(
        out.action,
        DecisionAction::StartTask | DecisionAction::InvokeSkill
    ) {
        let intent_ok = a.intent == Intent::Request
            || (out.action == DecisionAction::InvokeSkill
                && a.intent == Intent::FollowUp
                && a.continuity == Continuity::Continuing
                && (input.direct || linked_followup))
            || (input.information_lookup
                && matches!(a.intent, Intent::Question | Intent::FollowUp));
        if !intent_ok || a.confidence != Level::High || a.audience != Audience::Bot {
            suppress(out, trace, "参与判断不能确认这是对 Bot 的明确执行请求");
        } else if a.intent == Intent::FollowUp && !input.direct && !linked_followup {
            suppress(out, trace, "技能承接缺少同一人的近期回复关联");
        }
        return;
    }
    let group_social = social && input.is_group && a.audience == Audience::Group;
    if !input.direct && a.intent == Intent::FollowUp && !linked_followup && !group_social {
        suppress(out, trace, "隐式承接缺少同一人的近期回复关联");
        return;
    }
    // 模型声称 audience=bot 不能单独绕过频率限制；免冷却需要真实承接或协议/明确称呼。
    let directed = input.direct || linked_followup;
    if linked_followup {
        trace.notes.push("依据真实回复归属承接同一人的对话".into());
    }
    if input.is_group && a.intent == Intent::Acknowledgment && !linked_followup && !group_social {
        suppress(out, trace, "收尾附和不需要礼貌性再回一轮");
        return;
    }
    if input.is_group && !directed {
        if social {
            if a.audience != Audience::Group
                || a.confidence != Level::High
                || a.benefit == Level::Low
                || a.novelty == Novelty::Repeated
                || matches!(a.evidence, Evidence::Missing | Evidence::Conflicting)
            {
                suppress(out, trace, "接话缺少明确的公开话题、参与价值或事实依据");
                return;
            }
            let Some(activity) = input.activity else {
                suppress(out, trace, "缺少本轮群聊热度，不猜测参与时机");
                return;
            };
            if let Some(reason) = activity.social_block(a.intent == Intent::Sharing) {
                suppress(out, trace, reason);
                return;
            }
            let cooldown = activity.cooldown_secs();
            trace.cooldown_secs = Some(cooldown);
            if input.reply_age.is_some_and(|age| age < cooldown) {
                suppress(out, trace, "自然接话冷却中，给群友留出回应空间");
                return;
            }
            out.reply_len = ReplyLen::Short;
            out.mention = false;
            trace
                .notes
                .push("允许顺着公开群聊简短接话，不要求被叫到或提供新知识".into());
            return;
        }
        if !matches!(a.audience, Audience::Group | Audience::Bot)
            || a.confidence != Level::High
            || a.benefit != Level::High
            || a.novelty != Novelty::New
            || matches!(a.evidence, Evidence::Missing | Evidence::Conflicting)
            || matches!(a.intent, Intent::Acknowledgment | Intent::Banter)
        {
            suppress(out, trace, "主动参与缺少高把握的新帮助，保持安静");
            return;
        }
        let cooldown = if input.bot_bubbles_5min >= 6 || input.human_messages_30s >= 15 {
            300
        } else if input.bot_bubbles_5min <= 2 && input.human_messages_30s <= 4 {
            60
        } else {
            120
        };
        trace.cooldown_secs = Some(cooldown);
        if input.reply_age.is_some_and(|age| age < cooldown) {
            suppress(
                out,
                trace,
                "群聊参与冷却中，按繁忙程度和 Bot 发言量减少插话",
            );
            return;
        }
        out.reply_len = ReplyLen::Short;
        out.mention = false;
        trace.notes.push("允许一次简短、有新帮助的主动参与".into());
    }
    if out.action == DecisionAction::Reply {
        if a.confidence == Level::Low || a.audience == Audience::Unclear {
            out.reply_mode = ReplyMode::Clarify;
            out.reply_len = ReplyLen::Short;
        } else if matches!(a.evidence, Evidence::Missing | Evidence::Conflicting)
            && out.reply_mode != ReplyMode::Clarify
        {
            out.reply_mode = ReplyMode::StateUncertainty;
            out.reply_len = ReplyLen::Short;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn output() -> DecisionOutput {
        serde_json::from_value(json!({"action":"reply","mood":"calm","mention":true,"reply_len":"long","reason":"场景评估",
            "assessment":{"audience":"group","intent":"sharing","continuity":"new_topic","confidence":"high","benefit":"high","novelty":"new","evidence":"not_needed","evidence_msg_ids":[1]}})).unwrap()
    }
    fn input(ids: &BTreeSet<i64>) -> PolicyInput<'_> {
        PolicyInput {
            is_group: true,
            direct: false,
            protocol_direct: false,
            information_lookup: false,
            anchor_id: 1,
            visible_ids: ids,
            linked_reply_id: Some(2),
            reply_age: Some(70),
            bot_bubbles_5min: 0,
            human_messages_30s: 2,
            activity: None,
        }
    }
    #[test]
    fn implicit_answer_requires_own_linked_dialogue_and_visible_evidence() {
        let ids = BTreeSet::from([1, 2, 3]);
        let mut ctx = input(&ids);
        ctx.reply_age = Some(0);
        let mut out = output();
        let a = out.assessment.as_mut().unwrap();
        a.audience = Audience::Bot;
        a.intent = Intent::FollowUp;
        a.continuity = Continuity::Continuing;
        a.evidence_msg_ids = vec![1, 2];
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(
            out.action,
            DecisionAction::Reply,
            "同一个人的无问号简短回答可承接"
        );
        out.action = DecisionAction::Reply;
        ctx.linked_reply_id = Some(3);
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(
            out.action,
            DecisionAction::Ignore,
            "不能借另一个人的对话归属继续"
        );
        out.action = DecisionAction::Reply;
        ctx.direct = true;
        out.assessment.as_mut().unwrap().evidence_msg_ids.push(999);
        let mut trace = PolicyTrace::default();
        apply(&mut out, &ctx, &mut trace);
        assert_eq!(out.action, DecisionAction::Ignore);
        assert_eq!(trace.evidence_valid, Some(false));
    }
    #[test]
    fn participation_depends_on_help_and_load_instead_of_question_punctuation() {
        let ids = BTreeSet::from([1]);
        let mut ctx = input(&ids);
        let mut out = output();
        let mut trace = PolicyTrace::default();
        apply(&mut out, &ctx, &mut trace);
        assert_eq!(
            out.action,
            DecisionAction::Reply,
            "有新帮助的分享不必带问号"
        );
        assert_eq!(out.reply_len, ReplyLen::Short);
        assert!(!out.mention);
        assert_eq!(trace.cooldown_secs, Some(60));
        for (bubbles, humans, age, expected) in
            [(3, 8, 119, 120), (6, 1, 299, 300), (0, 15, 299, 300)]
        {
            ctx.bot_bubbles_5min = bubbles;
            ctx.human_messages_30s = humans;
            ctx.reply_age = Some(age);
            let mut out = output();
            let mut trace = PolicyTrace::default();
            apply(&mut out, &ctx, &mut trace);
            assert_eq!(out.action, DecisionAction::Ignore);
            assert_eq!(trace.cooldown_secs, Some(expected));
        }
        ctx.reply_age = None;
        let mut out = output();
        out.assessment.as_mut().unwrap().novelty = Novelty::Repeated;
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(out.action, DecisionAction::Ignore);
        let mut out = output();
        out.assessment.as_mut().unwrap().benefit = Level::Low;
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(out.action, DecisionAction::Ignore);
    }
    #[test]
    fn closing_and_uncertainty_have_distinct_behaviors() {
        let ids = BTreeSet::from([1]);
        let mut ctx = input(&ids);
        ctx.direct = true;
        ctx.reply_age = Some(0);
        let mut out = output();
        let a = out.assessment.as_mut().unwrap();
        a.audience = Audience::Bot;
        a.intent = Intent::Question;
        a.evidence = Evidence::Missing;
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(out.action, DecisionAction::Reply);
        assert_eq!(out.reply_mode, ReplyMode::StateUncertainty);
        out.assessment.as_mut().unwrap().confidence = Level::Low;
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(out.reply_mode, ReplyMode::Clarify);
        out.assessment.as_mut().unwrap().continuity = Continuity::Closing;
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(out.action, DecisionAction::Ignore);
        out.action = DecisionAction::Ignore;
        out.assessment.as_mut().unwrap().continuity = Continuity::NewTopic;
        apply(&mut out, &ctx, &mut PolicyTrace::default());
        assert_eq!(
            out.action,
            DecisionAction::Ignore,
            "不强制模型忽略的内容得到回复"
        );
    }
    #[test]
    fn social_chat_uses_heat_without_requiring_new_knowledge_or_own_reply_link() {
        let ids = BTreeSet::from([1]);
        let lively = Activity::from_counts(12, 3, 24, 10, 5, 4, 1);
        let warm = Activity::from_counts(6, 2, 16, 10, 3, 4, 1);
        let quiet = Activity::from_counts(1, 1, 1, 0, 1, 1, 0);
        let mut ctx = input(&ids);
        ctx.linked_reply_id = None;
        for (activity, age, intent, expected) in [
            (&lively, 50, Intent::Banter, DecisionAction::Reply),
            (&lively, 50, Intent::FollowUp, DecisionAction::Reply),
            (&warm, 50, Intent::Banter, DecisionAction::Ignore),
            (&warm, 90, Intent::Banter, DecisionAction::Reply),
            (&quiet, 200, Intent::Sharing, DecisionAction::Reply),
            (&quiet, 200, Intent::Acknowledgment, DecisionAction::Ignore),
        ] {
            ctx.activity = Some(activity);
            ctx.reply_age = Some(age);
            let mut out = output();
            out.reply_mode = ReplyMode::Social;
            let a = out.assessment.as_mut().unwrap();
            a.intent = intent;
            a.continuity = Continuity::Continuing;
            a.benefit = Level::Medium;
            a.novelty = Novelty::Unclear;
            apply(&mut out, &ctx, &mut PolicyTrace::default());
            assert_eq!(
                out.action, expected,
                "{:?} {age} {intent:?}",
                activity.level
            );
            assert_eq!(out.reply_len, ReplyLen::Short);
            assert!(!out.mention);
        }
    }

    #[test]
    fn social_chat_cannot_override_others_closing_source_or_execution_boundaries() {
        let ids = BTreeSet::from([1]);
        let lively = Activity::from_counts(12, 3, 24, 10, 5, 4, 1);
        let mut ctx = input(&ids);
        ctx.activity = Some(&lively);
        for case in [
            "other",
            "closing",
            "forged",
            "repeated",
            "task",
            "skill",
            "silent",
            "missing_assessment",
        ] {
            let mut out = output();
            out.reply_mode = ReplyMode::Social;
            let a = out.assessment.as_mut().unwrap();
            match case {
                "other" => a.audience = Audience::Other,
                "closing" => a.continuity = Continuity::Closing,
                "forged" => a.evidence_msg_ids.push(999),
                "repeated" => a.novelty = Novelty::Repeated,
                "task" => out.action = DecisionAction::StartTask,
                "skill" => out.action = DecisionAction::InvokeSkill,
                "silent" => out.action = DecisionAction::Ignore,
                "missing_assessment" => out.assessment = None,
                _ => unreachable!(),
            }
            out.task_goal = Some("不应执行".into());
            out.skill_name = Some("nai-image.generate".into());
            apply(&mut out, &ctx, &mut PolicyTrace::default());
            assert_eq!(out.action, DecisionAction::Ignore, "{case}");
            assert!(out.task_goal.is_none() && out.skill_name.is_none());
        }
    }
    #[test]
    fn protocol_target_corrects_only_a_grounded_request_misclassified_as_other() {
        let ids = BTreeSet::from([1]);
        let mut ctx = input(&ids);
        ctx.direct = true;
        ctx.protocol_direct = true;
        for case in [
            "request",
            "closing",
            "stop",
            "ack",
            "low",
            "forged",
            "normal_ignore",
            "ambiguous_mentions",
        ] {
            let mut out = output();
            out.action = DecisionAction::Ignore;
            let a = out.assessment.as_mut().unwrap();
            a.audience = Audience::Other;
            a.intent = Intent::Request;
            match case {
                "closing" => a.continuity = Continuity::Closing,
                "stop" => a.intent = Intent::Stop,
                "ack" => a.intent = Intent::Acknowledgment,
                "low" => a.confidence = Level::Low,
                "forged" => a.evidence_msg_ids.push(999),
                "normal_ignore" => a.audience = Audience::Bot,
                _ => (),
            }
            ctx.protocol_direct = case != "ambiguous_mentions";
            out.task_goal = Some("不应执行".into());
            out.skill_name = Some("nai-image.generate".into());
            let mut trace = PolicyTrace::default();
            apply(&mut out, &ctx, &mut trace);
            assert_eq!(
                out.action,
                if case == "request" {
                    DecisionAction::Reply
                } else {
                    DecisionAction::Ignore
                },
                "{case}"
            );
            if case == "request" {
                assert!(out.task_goal.is_none() && out.skill_name.is_none());
                assert_eq!(out.assessment.unwrap().audience, Audience::Bot);
                assert!(!trace.notes.is_empty());
            }
        }
    }
}
