//! Read-only GitHub polling for linked session notifications. No GitHub calls
//! occur until the shell invokes poll; acknowledgment persistence stays in shell.
use gpui::{Context, EventEmitter};
use monocode_core::{
    session_groups::WorkItem,
    store::{Session, Store},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};
const SEEN_KEY: &str = "linked-work-item-seen";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub id: String,
    pub kind: String,
    pub author: String,
    pub text: String,
    pub created_at: String,
    pub url: String,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Card {
    pub session: i64,
    pub item: WorkItem,
    pub title: String,
    pub state: String,
    pub updated_at: i64,
    pub entries: Vec<Entry>,
    pub details_error: bool,
    pub truncated: bool,
}
impl Card {
    pub(crate) fn summary(&self) -> String {
        let count = |kind: &str| {
            self.entries
                .iter()
                .filter(|e| {
                    if kind == "comment" {
                        e.kind == "comment" || e.kind == "review_comment"
                    } else {
                        e.kind == kind
                    }
                })
                .count()
        };
        let parts = [
            (count("commit"), "commit"),
            (count("review"), "review"),
            (count("comment"), "comment"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, k)| format!("{n} new {k}{}", if n == 1 { "" } else { "s" }))
        .collect::<Vec<_>>();
        if !parts.is_empty() {
            parts.join(" · ")
        } else if self.details_error {
            "Updated on GitHub · details unavailable".into()
        } else {
            "Metadata or status changed".into()
        }
    }
    pub(crate) fn terminal_label(&self) -> Option<&'static str> {
        match (
            self.item.kind.as_str(),
            self.state.to_ascii_lowercase().as_str(),
        ) {
            ("pr", "merged") => Some("Pull request merged"),
            ("pr", "closed") => Some("Pull request closed"),
            ("issue", "closed") => Some("Issue closed"),
            _ => None,
        }
    }
    pub(crate) fn prompt(&self) -> String {
        let first = self.entries.first();
        let instruction = match first.map(|e| e.kind.as_str()) {
            Some("commit") => "Review the new commit and continue the work where needed.",
            Some(_) => "Address the new feedback and continue the work where needed.",
            None => "Review the latest update and continue the work where needed.",
        };
        let details = if self.entries.is_empty() {
            vec![format!(
                "- {}; current state: {}",
                self.summary(),
                self.state
            )]
        } else {
            self.entries
                .iter()
                .take(12)
                .map(|e| {
                    format!(
                        "- {}{}: {}",
                        e.kind.replace('_', " "),
                        if e.author.is_empty() {
                            String::new()
                        } else {
                            format!(" by @{}", e.author)
                        },
                        if e.text.is_empty() {
                            "No message"
                        } else {
                            &e.text
                        }
                    )
                })
                .collect()
        };
        format!(
            "{instruction}\n\nThe linked GitHub {} has new activity:\n\n#{} {}\n{}\n\n{}",
            if self.item.kind == "pr" {
                "pull request"
            } else {
                "issue"
            },
            self.item.number,
            self.title,
            self.item.url,
            details.join("\n")
        )
    }
}
pub(crate) fn load_seen(store: &Store) -> Result<HashMap<i64, i64>, String> {
    Ok(store
        .setting(SEEN_KEY)?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}
/// Merge the desktop's acknowledged UUID snapshots with the native cache.
/// Resolve source IDs through Store rather than treating UUIDs as native IDs.
pub(crate) fn load_seen_with_metadata(
    store: &Store,
    metadata: Option<&Value>,
) -> Result<HashMap<i64, i64>, String> {
    let mut seen = load_seen(store)?;
    let raw = metadata.and_then(|m| m.get("monocode.linkedSessionSeen"));
    let decoded = raw
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok());
    let values = decoded.as_ref().or(raw).and_then(Value::as_object);
    if let Some(values) = values {
        for (source_id, timestamp) in values {
            if let (Some(id), Some(at)) = (
                store.resolve_existing_session_id(source_id),
                timestamp.as_i64(),
            ) {
                let current = seen.entry(id).or_default();
                *current = (*current).max(at);
            }
        }
    }
    Ok(seen)
}
pub(crate) fn save_seen(store: &Store, seen: &HashMap<i64, i64>) -> Result<(), String> {
    store.save_setting(
        SEEN_KEY,
        &serde_json::to_string(seen).map_err(|e| e.to_string())?,
    )
}
pub(crate) enum GithubActivityEvent {
    Changed,
    Acknowledged(HashMap<i64, i64>),
}
pub(crate) struct GithubActivity {
    pub cards: HashMap<i64, Card>,
    seen: HashMap<i64, i64>,
    pull_requests_after: Option<i64>,
    checked: Option<Instant>,
    checking: bool,
    /// Detail feeds by URL with the item `updated_at` they were fetched for:
    /// an unchanged item reuses them instead of calling gh every poll.
    details: Details,
}
type Details = HashMap<String, (i64, Value)>;
impl EventEmitter<GithubActivityEvent> for GithubActivity {}
impl GithubActivity {
    pub(crate) fn new(pull_requests_after: Option<i64>) -> Self {
        Self {
            cards: HashMap::new(),
            seen: HashMap::new(),
            pull_requests_after,
            checked: None,
            checking: false,
            details: HashMap::new(),
        }
    }
    pub(crate) fn set_pull_requests_after(&mut self, after: Option<i64>) {
        if self.pull_requests_after != after {
            self.pull_requests_after = after;
            self.cards.retain(|_, card| card.item.kind != "pr");
        }
    }
    pub(crate) fn restore_seen(&mut self, seen: HashMap<i64, i64>) {
        for (id, at) in seen {
            let current = self.seen.entry(id).or_default();
            *current = (*current).max(at);
        }
        self.cards
            .retain(|id, card| card.updated_at > *self.seen.get(id).unwrap_or(&0));
    }
    pub(crate) fn acknowledge(&mut self, session: i64, cx: &mut Context<Self>) {
        if let Some(card) = self.cards.remove(&session) {
            let seen = self.seen.entry(session).or_default();
            *seen = (*seen).max(card.updated_at);
            if self.seen.len() > 500 {
                let mut entries: Vec<_> = self.seen.iter().map(|(id, at)| (*id, *at)).collect();
                entries.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
                self.seen = entries.into_iter().take(500).collect();
            }
            cx.emit(GithubActivityEvent::Acknowledged(self.seen.clone()));
            cx.emit(GithubActivityEvent::Changed);
            cx.notify();
        }
    }
    pub(crate) fn ready_to_poll(&self) -> bool {
        !self.checking
            && self
                .checked
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(60))
    }
    pub(crate) fn poll(&mut self, sessions: Vec<Session>, cx: &mut Context<Self>) {
        if self.checking
            || self
                .checked
                .is_some_and(|at| at.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        self.checking = true;
        self.checked = Some(Instant::now());
        let seen = self.seen.clone();
        let pull_requests_after = self.pull_requests_after;
        let details = std::mem::take(&mut self.details);
        let work = cx
            .background_executor()
            .spawn(async move { fetch(sessions, &seen, details, pull_requests_after) });
        cx.spawn(async move |this, cx| {
            let (cards, details) = work.await;
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                this.details = details;
                this.replace_cards(cards, pull_requests_after);
                cx.emit(GithubActivityEvent::Changed);
                cx.notify();
            });
        })
        .detach();
    }

    fn replace_cards(&mut self, cards: HashMap<i64, Card>, polled_after: Option<i64>) {
        // A pending poll can finish after the PR setting changes.
        self.cards = cards
            .into_iter()
            .filter(|(id, card)| {
                card.updated_at > *self.seen.get(id).unwrap_or(&0)
                    && (card.item.kind != "pr"
                        || self
                            .pull_requests_after
                            .is_some_and(|after| polled_after == Some(after)))
            })
            .collect();
    }
}
fn timestamp(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|d| d.timestamp_millis())
        .unwrap_or(0)
}
fn api(endpoint: &str) -> Result<Value, String> {
    let binary = monocode_core::scm::resolve_gui_binary("gh").ok_or("GitHub CLI not found")?;
    let text = monocode_core::cli::output(
        &binary,
        &[
            "api".into(),
            "--method".into(),
            "GET".into(),
            endpoint.into(),
            "--jq".into(),
            r#"if type == "array" then map({id,sha,user:{login:.user.login},author:{login:.author.login},body:((.body // "")[0:256]),state,submitted_at,created_at,html_url,commit:{message:((.commit.message // "")[0:256]),committer:{date:.commit.committer.date}}}) else {title,state,merged,updated_at} end"#.into(),
        ],
        Path::new("/"),
        Duration::from_secs(20),
    )?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
/// Only canonical GitHub repository slugs enter a gh API endpoint or query.
fn canonical(item: &WorkItem) -> bool {
    let parts = item.repo.split('/').collect::<Vec<_>>();
    item.number != 0
        && parts.len() == 2
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        })
}

fn activity_since(item: &WorkItem, since: i64, pull_requests_after: Option<i64>) -> Option<i64> {
    if item.kind == "pr" {
        pull_requests_after.map(|after| since.max(after))
    } else {
        Some(since)
    }
}

/// Every linked item's summary in one GraphQL request, shaped like the REST
/// `api` summary. None if the request fails, so callers fall back to REST.
fn summaries(items: &[(String, u64)]) -> Option<HashMap<(String, u64), Value>> {
    let mut out = HashMap::new();
    for chunk in items.chunks(50) {
        let mut query = String::from("query{");
        for (ix, (repo, number)) in chunk.iter().enumerate() {
            let (owner, name) = repo.split_once('/')?;
            query.push_str(&format!(
                "i{ix}:repository(owner:\"{owner}\",name:\"{name}\"){{issueOrPullRequest(number:{number}){{...on PullRequest{{title state merged updatedAt}}...on Issue{{title state updatedAt}}}}}}"
            ));
        }
        query.push('}');
        let binary = monocode_core::scm::resolve_gui_binary("gh")?;
        let text = monocode_core::cli::output(
            &binary,
            &[
                "api".into(),
                "graphql".into(),
                "-f".into(),
                format!("query={query}"),
            ],
            Path::new("/"),
            Duration::from_secs(20),
        )
        .ok()?;
        let body: Value = serde_json::from_str(&text).ok()?;
        let data = body.get("data")?;
        for (ix, item) in chunk.iter().enumerate() {
            let node = &data[format!("i{ix}")]["issueOrPullRequest"];
            if node.is_null() {
                continue;
            }
            // REST reports a merged pull request as closed plus `merged`.
            let state = match node["state"].as_str().unwrap_or("") {
                "MERGED" => "closed".to_owned(),
                state => state.to_ascii_lowercase(),
            };
            out.insert(
                item.clone(),
                serde_json::json!({
                    "title": node["title"],
                    "state": state,
                    "merged": node["merged"].as_bool().unwrap_or(false),
                    "updated_at": node["updatedAt"],
                }),
            );
        }
    }
    Some(out)
}
fn fetch(
    sessions: Vec<Session>,
    seen: &HashMap<i64, i64>,
    previous: Details,
    pull_requests_after: Option<i64>,
) -> (HashMap<i64, Card>, Details) {
    let mut cards = HashMap::new();
    let mut details = Details::new();
    let sessions: Vec<_> = sessions
        .into_iter()
        .filter_map(|session| {
            let item = session
                .work_item
                .filter(|item| !session.archived && canonical(item))?;
            let since = (session.updated_at.min(i64::MAX as u64) as i64)
                .max(*seen.get(&session.id).unwrap_or(&0));
            let since = activity_since(&item, since, pull_requests_after)?;
            Some((session.id, item, since))
        })
        .collect();
    let mut items: Vec<(String, u64)> = sessions
        .iter()
        .map(|(_, item, _)| (item.repo.clone(), item.number))
        .collect();
    items.sort();
    items.dedup();
    let batched = summaries(&items);
    let mut cache: HashMap<String, Option<Value>> = HashMap::new();
    for (session, item, since) in sessions {
        let base = format!(
            "repos/{}/{}s/{}",
            item.repo,
            if item.kind == "pr" { "pull" } else { "issue" },
            item.number
        );
        let summary = match &batched {
            Some(batched) => batched.get(&(item.repo.clone(), item.number)).cloned(),
            None => cache
                .entry(base.clone())
                .or_insert_with(|| api(&base).ok())
                .clone(),
        };
        let Some(body) = summary else {
            continue;
        };
        let updated_at = timestamp(body["updated_at"].as_str().unwrap_or(""));
        if updated_at <= since {
            continue;
        }
        let mut card = Card {
            session,
            title: body["title"].as_str().unwrap_or("").into(),
            state: if body["merged"].as_bool() == Some(true) {
                "merged".into()
            } else {
                body["state"].as_str().unwrap_or("").into()
            },
            updated_at,
            item,
            entries: Vec::new(),
            details_error: false,
            truncated: false,
        };
        let issue_base = format!("repos/{}/issues/{}", card.item.repo, card.item.number);
        let mut feeds = vec![(format!("{issue_base}/comments?per_page=100"), "comment")];
        if card.item.kind == "pr" {
            feeds.extend([
                (format!("{base}/reviews?per_page=100"), "review"),
                (format!("{base}/comments?per_page=100"), "review_comment"),
                (format!("{base}/commits?per_page=100"), "commit"),
            ]);
        }
        for (url, kind) in feeds {
            let body = match previous.get(&url).or_else(|| details.get(&url)) {
                Some((at, body)) if *at == updated_at => body.clone(),
                _ => match api(&url) {
                    Ok(body) => body,
                    Err(_) => {
                        card.details_error = true;
                        continue;
                    }
                },
            };
            details.insert(url, (updated_at, body.clone()));
            let Some(entries) = body.as_array() else {
                card.details_error = true;
                continue;
            };
            card.truncated |= entries.len() == 100;
            for entry in entries {
                let commit = kind == "commit";
                let at = if commit {
                    entry["commit"]["committer"]["date"].as_str()
                } else if kind == "review" {
                    entry["submitted_at"].as_str()
                } else {
                    entry["created_at"].as_str()
                }
                .unwrap_or("");
                if timestamp(at) <= since {
                    continue;
                }
                let text = if commit {
                    entry["commit"]["message"]
                        .as_str()
                        .unwrap_or("")
                        .lines()
                        .next()
                        .unwrap_or("")
                        .into()
                } else {
                    let body = entry["body"].as_str().unwrap_or("");
                    if kind == "review" {
                        format!(
                            "{}: {body}",
                            entry["state"].as_str().unwrap_or("").replace('_', " ")
                        )
                    } else {
                        body.to_owned()
                    }
                };
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let text = if text.chars().count() > 140 {
                    format!("{}…", text.chars().take(139).collect::<String>())
                } else {
                    text
                };
                card.entries.push(Entry {
                    id: if commit {
                        entry["sha"].as_str().unwrap_or("").into()
                    } else {
                        entry["id"].to_string()
                    },
                    kind: kind.into(),
                    author: entry["user"]["login"]
                        .as_str()
                        .or_else(|| entry["author"]["login"].as_str())
                        .unwrap_or("")
                        .into(),
                    text,
                    created_at: at.into(),
                    url: entry["html_url"].as_str().unwrap_or("").into(),
                });
            }
        }
        card.entries
            .sort_by_key(|e| std::cmp::Reverse(timestamp(&e.created_at)));
        cards.insert(card.session, card);
    }
    (cards, details)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity_card(session: i64, kind: &str, updated_at: i64) -> Card {
        Card {
            session,
            item: WorkItem {
                kind: kind.into(),
                repo: "owner/repo".into(),
                number: 1,
                url: String::new(),
            },
            title: "Updated item".into(),
            state: "open".into(),
            updated_at,
            entries: Vec::new(),
            details_error: false,
            truncated: false,
        }
    }

    #[test]
    fn linked_pr_poll_uses_resume_time_for_activity_and_details() {
        let pull_request = activity_card(1, "pr", 0).item;
        let issue = activity_card(2, "issue", 0).item;
        assert_eq!(activity_since(&pull_request, 10, None), None);
        assert_eq!(activity_since(&pull_request, 10, Some(20)), Some(20));
        assert_eq!(activity_since(&pull_request, 30, Some(20)), Some(30));
        assert_eq!(activity_since(&issue, 10, None), Some(10));
        assert_eq!(activity_since(&issue, 10, Some(20)), Some(10));
    }

    #[test]
    fn linked_pr_poll_drops_stale_results_and_keeps_issues() {
        let mut feed = GithubActivity::new(Some(0));
        let cards = HashMap::from([
            (1, activity_card(1, "pr", 30)),
            (2, activity_card(2, "issue", 30)),
        ]);
        feed.replace_cards(cards.clone(), Some(0));
        assert_eq!(feed.cards.len(), 2);

        feed.set_pull_requests_after(None);
        assert!(!feed.cards.contains_key(&1));
        assert!(feed.cards.contains_key(&2));
        feed.replace_cards(cards.clone(), Some(0));
        assert!(!feed.cards.contains_key(&1));
        assert!(feed.cards.contains_key(&2));

        feed.set_pull_requests_after(Some(20));
        feed.replace_cards(cards.clone(), Some(0));
        assert!(!feed.cards.contains_key(&1));
        assert!(feed.cards.contains_key(&2));
        feed.replace_cards(cards.clone(), None);
        assert!(!feed.cards.contains_key(&1));
        assert!(feed.cards.contains_key(&2));
        feed.replace_cards(cards, Some(20));
        assert_eq!(feed.cards.len(), 2);
    }
}
