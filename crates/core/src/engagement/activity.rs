//! Conversation activity is evidence about timing, not permission to speak or act.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Heat {
    Quiet,
    Warming,
    Lively,
    Crowded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Activity {
    pub level: Heat,
    pub human_messages_2min: i64,
    pub active_speakers_2min: i64,
    pub human_messages_5min: i64,
    pub previous_messages_5min: i64,
    pub dominant_speaker_messages_2min: i64,
    pub human_messages_since_bot: i64,
    pub bot_turns_5min: i64,
    pub trend: Trend,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    Rising,
    Steady,
    Fading,
}

impl Activity {
    pub fn from_counts(
        messages: i64,
        speakers: i64,
        humans_5min: i64,
        previous: i64,
        dominant: i64,
        since_bot: i64,
        bot_turns: i64,
    ) -> Self {
        let level = if messages >= 40 {
            Heat::Crowded
        } else if messages >= 10 && speakers >= 3 {
            Heat::Lively
        } else if messages >= 4 && speakers >= 2 {
            Heat::Warming
        } else {
            Heat::Quiet
        };
        let trend = if messages >= 4 && messages * 5 > previous * 3 {
            Trend::Rising
        } else if previous >= 6 && messages * 5 < previous {
            Trend::Fading
        } else {
            Trend::Steady
        };
        Self {
            level,
            human_messages_2min: messages,
            active_speakers_2min: speakers,
            human_messages_5min: humans_5min,
            previous_messages_5min: previous,
            dominant_speaker_messages_2min: dominant,
            human_messages_since_bot: since_bot,
            bot_turns_5min: bot_turns,
            trend,
        }
    }

    pub fn cooldown_secs(&self) -> i64 {
        match self.level {
            Heat::Quiet => 180,
            Heat::Warming => 90,
            Heat::Lively => 45,
            Heat::Crowded => 90,
        }
    }

    pub fn social_block(&self, is_public_sharing: bool) -> Option<&'static str> {
        if self.bot_turns_5min >= 4 {
            return Some("近5分钟已发言4轮，让群友继续聊");
        }
        if self.bot_turns_5min >= 2 && self.bot_turns_5min * 4 > self.human_messages_5min {
            return Some("云团在近期对话中占比偏高，先听群友聊");
        }
        if self.human_messages_2min >= 12
            && self.dominant_speaker_messages_2min * 5 >= self.human_messages_2min * 4
        {
            return Some("消息主要来自同一人的连续发言，等待表达完整");
        }
        let minimum = match self.level {
            Heat::Quiet => 1,
            Heat::Warming => 2,
            Heat::Lively => 3,
            Heat::Crowded => 6,
        };
        if self.human_messages_since_bot < minimum {
            return Some("上一轮后新增的人类发言不足，不接自己的话");
        }
        if self.level == Heat::Quiet && !(is_public_sharing && self.human_messages_2min <= 3) {
            return Some("当前缺少共同话题的交流，只接有明确落点的公开分享");
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn heat_needs_people_and_crowding_does_not_mean_unbounded_talking() {
        let warm = Activity::from_counts(5, 2, 10, 5, 3, 3, 0);
        let lively = Activity::from_counts(12, 3, 22, 8, 5, 5, 1);
        let crowded = Activity::from_counts(45, 8, 70, 20, 10, 8, 1);
        assert_eq!(
            (warm.level, lively.level, crowded.level),
            (Heat::Warming, Heat::Lively, Heat::Crowded)
        );
        assert!(lively.cooldown_secs() < warm.cooldown_secs());
        assert!(crowded.cooldown_secs() > lively.cooldown_secs());
        assert!(lively.social_block(false).is_none());
        let flood = Activity::from_counts(30, 1, 30, 0, 30, 30, 0);
        assert!(flood.social_block(true).is_some());
    }
    #[test]
    fn silence_bot_monologue_and_bot_share_are_separate_from_a_public_share() {
        let mut a = Activity::from_counts(1, 1, 1, 0, 1, 1, 0);
        assert!(a.social_block(false).is_some());
        assert!(a.social_block(true).is_none());
        a.human_messages_since_bot = 0;
        assert!(a.social_block(true).is_some());
        a.human_messages_since_bot = 1;
        a.bot_turns_5min = 2;
        assert!(a.social_block(true).is_some());
        a.bot_turns_5min = 4;
        a.human_messages_5min = 100;
        assert!(a.social_block(true).is_some());
    }
}
