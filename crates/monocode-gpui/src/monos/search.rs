use super::*;

impl Monos {
    pub(crate) fn search_sources(
        &self,
    ) -> Result<(HashMap<i64, crate::search::MonoIdentity>, Vec<Session>), String> {
        let store = locked(&self.store)?;
        let mut identities = HashMap::new();
        let mut summaries = Vec::new();
        for mono in &self.roster.monos {
            if let Some(session) = mono.session_id {
                identities.insert(session, crate::search::MonoIdentity::from(mono));
                if let Some(mut summary) = store.mono_session(session)? {
                    summary.title = mono.label();
                    summaries.push(summary);
                }
            }
        }
        Ok((identities, summaries))
    }
    pub(crate) fn search_conversations(&self) -> Vec<Entity<Conversation>> {
        self.chats
            .values()
            .chain(self.running.values().map(|r| &r.view))
            .cloned()
            .collect()
    }
    pub(crate) fn search_session_access(&self, session: i64) -> Result<bool, String> {
        if self.roster.of_session(session).is_none() {
            return Ok(false);
        }
        if !self.prefs.monos {
            return Err("Turn on Show monos in Settings → Monos to open this conversation.".into());
        }
        Ok(true)
    }
    pub(crate) fn open_search_session(
        &mut self,
        session: i64,
        jump: Option<(usize, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        if !self.search_session_access(session)? {
            return Ok(false);
        }
        let Some(mono) = self.roster.of_session(session) else {
            return Ok(false);
        };
        let id = mono.id.clone();
        self.search_jumps.remove(&session);
        self.open(&id, window, cx);
        if let Some((row, query)) = jump {
            if let Some(view) = self.chats.get(&id) {
                view.update(cx, |view, cx| view.reveal_search(row, query, cx));
            } else {
                self.search_jumps.insert(session, (row, query));
            }
        }
        Ok(true)
    }
}
