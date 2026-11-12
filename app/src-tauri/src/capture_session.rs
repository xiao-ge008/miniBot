//! 截图会话状态机：隔离重复热键、过期前端回调和超时任务。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Capturing,
    Preparing,
    Visible,
}

#[derive(Debug)]
struct Session {
    id: String,
    phase: Phase,
}

#[derive(Debug, Default)]
pub struct CaptureLifecycle {
    session: Option<Session>,
}

impl CaptureLifecycle {
    pub fn begin(&mut self, id: &str) -> bool {
        if self.session.is_some() {
            return false;
        }
        self.session = Some(Session {
            id: id.to_owned(),
            phase: Phase::Capturing,
        });
        true
    }

    pub fn is_active(&self, id: &str) -> bool {
        self.session.as_ref().is_some_and(|s| s.id == id)
    }

    pub fn current_id(&self) -> Option<String> {
        self.session.as_ref().map(|s| s.id.clone())
    }

    pub fn prepare(&mut self, id: &str) -> bool {
        self.transition(id, Phase::Capturing, Phase::Preparing)
    }

    pub fn show(&mut self, id: &str) -> bool {
        self.transition(id, Phase::Preparing, Phase::Visible)
    }

    pub fn is_preparing(&self, id: &str) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.id == id && s.phase == Phase::Preparing)
    }

    pub fn close(&mut self, id: &str) -> bool {
        if !self.is_active(id) {
            return false;
        }
        self.session = None;
        true
    }

    fn transition(&mut self, id: &str, from: Phase, to: Phase) -> bool {
        if let Some(s) = self.session.as_mut() {
            if s.id == id && s.phase == from {
                s.phase = to;
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_reentry_before_frame_is_ready() {
        let mut state = CaptureLifecycle::default();
        assert!(state.begin("first"));
        assert!(!state.begin("repeated-hotkey"));
        assert_eq!(state.current_id().as_deref(), Some("first"));
    }

    #[test]
    fn cannot_show_before_decoding_and_only_shows_once() {
        let mut state = CaptureLifecycle::default();
        assert!(state.begin("first"));
        assert!(!state.show("first"));
        assert!(state.prepare("first"));
        assert!(state.is_preparing("first"));
        assert!(state.show("first"));
        assert!(!state.show("first"));
        assert!(!state.is_preparing("first"));
    }

    #[test]
    fn stale_callbacks_cannot_show_or_close_next_capture() {
        let mut state = CaptureLifecycle::default();
        assert!(state.begin("old"));
        assert!(state.prepare("old"));
        assert!(state.close("old"));
        assert!(state.begin("new"));
        assert!(state.prepare("new"));
        assert!(!state.show("old"));
        assert!(!state.close("old"));
        assert!(!state.is_preparing("old"));
        assert!(state.show("new"));
    }

    #[test]
    fn closing_a_failed_capture_allows_retry() {
        let mut state = CaptureLifecycle::default();
        assert!(state.begin("failed"));
        assert!(state.close("failed"));
        assert!(!state.prepare("failed"));
        assert!(state.begin("retry"));
    }
}
