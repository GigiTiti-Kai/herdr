use std::time::Instant;

use crate::detect::Agent;

use super::TerminalState;

impl TerminalState {
    pub(crate) fn background_shells(&self) -> Option<u32> {
        let (agent, count) = self.background_shells?;
        if self.effective_known_agent() != Some(agent)
            || self.recent_agent_process_exit.is_some_and(|exit| {
                exit.agent == agent
                    && self
                        .background_shells_observed_at
                        .is_none_or(|observed_at| observed_at <= exit.observed_at)
            })
        {
            return None;
        }
        Some(count)
    }

    pub(crate) fn set_background_shells_at(
        &mut self,
        agent: Option<Agent>,
        count: Option<u32>,
        observed_at: Instant,
    ) -> bool {
        if self
            .background_shells_observed_at
            .is_some_and(|previous| observed_at < previous)
            || self
                .fallback_observed_at
                .is_some_and(|previous| observed_at < previous)
            || (agent.is_some() && agent != self.effective_known_agent())
            || self.recent_agent_process_exit.is_some_and(|exit| {
                agent == Some(exit.agent) && count.is_some() && observed_at <= exit.observed_at
            })
        {
            return false;
        }
        let next = agent.zip(count.filter(|count| *count > 0));
        self.background_shells_observed_at = Some(observed_at);
        if self.background_shells == next {
            return false;
        }
        self.background_shells = next;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub(crate) fn agent_tokens(&self) -> std::collections::HashMap<String, String> {
        let mut tokens = self.metadata_tokens.values();
        // This name belongs to current runtime evidence, not an external reporter.
        tokens.remove("background_shells");
        if let Some(count) = self.background_shells() {
            tokens.insert(
                "background_shells".into(),
                format!("{count} {}", if count == 1 { "shell" } else { "shells" }),
            );
        }
        tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::AgentState;
    use crate::terminal::TerminalId;
    use std::time::Duration;

    fn terminal_at(now: Instant) -> TerminalState {
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        terminal.set_detected_state_with_screen_signals_at(
            Some(Agent::Claude),
            AgentState::Working,
            false,
            false,
            true,
            false,
            now,
        );
        terminal.last_agent_state_change_seq = Some(12);
        terminal.last_agent_completion_seq = Some(9);
        terminal
    }

    #[test]
    fn background_shells_changes_do_not_change_response_lifecycle() {
        let now = Instant::now();
        let mut terminal = terminal_at(now);
        for (step, count) in [Some(1), Some(2), None].into_iter().enumerate() {
            assert!(terminal.set_background_shells_at(
                Some(Agent::Claude),
                count,
                now + Duration::from_millis(step as u64 + 1),
            ));
            assert_eq!(terminal.background_shells(), count);
            assert_eq!(terminal.state, AgentState::Working);
            assert_eq!(terminal.last_agent_state_change_seq, Some(12));
            assert_eq!(terminal.last_agent_completion_seq, Some(9));
        }
        assert!(!terminal.set_background_shells_at(
            Some(Agent::Claude),
            Some(0),
            now + Duration::from_millis(5),
        ));
        assert_eq!(terminal.background_shells(), None);
    }

    #[test]
    fn background_shells_reject_stale_and_foreign_observations() {
        let now = Instant::now();
        let mut terminal = terminal_at(now);
        assert!(terminal.set_background_shells_at(
            Some(Agent::Claude),
            Some(2),
            now + Duration::from_millis(2),
        ));
        assert!(!terminal.set_background_shells_at(Some(Agent::Claude), Some(1), now));
        assert!(!terminal.set_background_shells_at(
            Some(Agent::Codex),
            Some(10),
            now + Duration::from_millis(3),
        ));
        assert_eq!(terminal.background_shells(), Some(2));
    }

    #[test]
    fn background_shell_new_process_of_same_kind_does_not_inherit_count() {
        let now = Instant::now();
        let mut terminal = terminal_at(now);
        terminal.set_background_shells_at(
            Some(Agent::Claude),
            Some(2),
            now + Duration::from_millis(1),
        );
        terminal.set_detected_agent_process_at(Agent::Claude, now + Duration::from_millis(2));
        assert_eq!(terminal.background_shells(), None);
        assert!(!terminal.set_background_shells_at(
            Some(Agent::Claude),
            Some(2),
            now + Duration::from_millis(1),
        ));
    }

    #[test]
    fn background_shells_are_not_reused_after_agent_replacement_or_respawn() {
        let now = Instant::now();
        let mut terminal = terminal_at(now);
        terminal.set_background_shells_at(Some(Agent::Claude), Some(2), now);
        assert_eq!(terminal.background_shells(), Some(2));
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Idle);
        assert_eq!(terminal.background_shells(), None);
        terminal.set_detected_state(Some(Agent::Claude), AgentState::Idle);
        assert_eq!(terminal.background_shells(), None);

        terminal.set_background_shells_at(Some(Agent::Claude), Some(1), Instant::now());
        assert_eq!(terminal.background_shells(), Some(1));
        terminal.clear_agent_runtime_identity_after_respawn();
        assert_eq!(terminal.background_shells(), None);
    }
}
