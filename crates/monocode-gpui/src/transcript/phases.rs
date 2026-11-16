//! Reasoning and calls share a phase until assistant prose starts the next one.
use monocode_core::{
    tool::{ToolCall, ToolState, tasks::Panel},
    transcript::Row,
};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(crate) struct Phases {
    groups: Vec<Group>,
    messages: HashMap<usize, usize>,
    pub hidden: HashSet<usize>,
    /// Delegated runs stay outside the work fold until their turn settles.
    /// Failed or cancelled runs retain that row so their report stays visible.
    pub pinned: HashSet<usize>,
}
pub(crate) struct Group {
    pub start: usize,
    pub end: usize,
    pub last: usize,
    pub steps: usize,
    pub calls: Vec<ToolCall>,
}
impl Phases {
    pub fn at(&self, message: usize) -> Option<&Group> {
        self.messages.get(&message).map(|&i| &self.groups[i])
    }
}

pub(crate) fn collect(rows: &[Row], from: usize, streaming: bool) -> Phases {
    collect_visible(rows, from, streaming, |_| false)
}

#[derive(serde::Deserialize)]
struct CallState {
    state: ToolState,
}

pub(crate) fn collect_visible(
    rows: &[Row],
    from: usize,
    streaming: bool,
    mut folded: impl FnMut(usize) -> bool,
) -> Phases {
    let mut result = Phases::default();
    let mut members = Vec::new();
    let mut calls = Vec::new();
    let live_turn = streaming.then(|| rows.iter().rposition(Row::is_prompt).map_or(0, |i| i + 1));
    let turn = rows[..from]
        .iter()
        .rposition(Row::is_prompt)
        .map_or(0, |i| i + 1);
    let mut leading = !rows[turn..from]
        .iter()
        .any(|r| r.role == "tool" || (r.role == "assistant" && !r.text.trim().is_empty()));
    let flush = |end, members: &mut Vec<usize>, calls: &mut Vec<ToolCall>, result: &mut Phases| {
        if let (Some(&start), Some(&last)) = (members.first(), members.last()) {
            for &at in members.iter() {
                result.messages.insert(at, result.groups.len());
            }
            result.groups.push(Group {
                start,
                end,
                last,
                steps: members.len(),
                calls: std::mem::take(calls),
            });
            members.clear();
        }
    };
    // Only rebuild the changed suffix; older tool payloads stay virtualized.
    let mut at = from;
    while at < rows.len() {
        let end = (at + 1..rows.len())
            .find(|&i| rows[i].first)
            .unwrap_or(rows.len());
        let role = rows[at].role.as_str();
        // Failed and cancelled agent reports stay visible inside closed work.
        if role == "tool"
            && folded(at)
            && live_turn.is_none_or(|turn| at < turn)
            && serde_json::from_str::<CallState>(&rows[at].text)
                .is_ok_and(|call| !matches!(call.state, ToolState::Failed | ToolState::Cancelled))
        {
            leading = false;
            at = end;
            continue;
        }
        let readable = rows[at..end].iter().any(|r| !r.text.trim().is_empty());
        if matches!(role, "reasoning" | "assistant") && !readable {
            result.hidden.insert(at);
        } else if role == "reasoning" {
            members.push(at);
        } else if role == "tool"
            && let Ok(mut call) = serde_json::from_str::<ToolCall>(&rows[at].text)
        {
            let standalone = match call.panel() {
                Some(Panel::Agent(_))
                    if live_turn.is_some_and(|turn| at >= turn)
                        || matches!(call.state, ToolState::Failed | ToolState::Cancelled) =>
                {
                    result.pinned.insert(at);
                    true
                }
                Some(Panel::Tasks(_)) => true,
                _ => false,
            };
            if standalone {
                flush(at, &mut members, &mut calls, &mut result);
                leading = false;
                at = end;
                continue;
            }
            // Keep compaction and handoffs visible as their own step instead of
            // burying them in an unrelated group of reads, commands or reasoning.
            let compaction = call.is_milestone();
            if compaction {
                flush(at, &mut members, &mut calls, &mut result);
            }
            members.push(at);
            // Summaries never need command output; do not retain another copy.
            call.output = String::new();
            if let Some(Panel::Agent(run)) = &mut call.panel {
                run.steps.clear();
            }
            calls.push(call);
            leading = false;
            if compaction {
                flush(end, &mut members, &mut calls, &mut result);
            }
        } else if role == "visual" {
            // A visual published mid-work belongs to that work; it must not
            // split the group around the call that published it.
        } else {
            // React's first assistant message replaces the initial private think.
            if role == "assistant" && leading && calls.is_empty() {
                result.hidden.extend(members.drain(..));
            }
            flush(at, &mut members, &mut calls, &mut result);
            if rows[at].is_prompt() {
                leading = true;
            } else if role != "system" {
                leading = false;
            }
        }
        at = end;
    }
    flush(rows.len(), &mut members, &mut calls, &mut result);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn build(rows: &[Row]) -> Phases {
        collect(rows, 0, false)
    }
    fn row(role: &str, text: &str) -> Row {
        Row {
            role: role.into(),
            text: text.into(),
            first: true,
            sent_at: None,
        }
    }
    fn tool() -> Row {
        row(
            "tool",
            &serde_json::to_string(&ToolCall::new("read", "Read")).unwrap(),
        )
    }
    #[test]
    fn empty_thoughts_are_invisible_and_do_not_split_calls() {
        let phases = build(&[
            row("user", "Check"),
            tool(),
            row("reasoning", " \n"),
            tool(),
        ]);
        assert!(phases.hidden.contains(&2));
        let group = phases.at(1).unwrap();
        assert_eq!(
            (group.start, group.last, group.steps, group.calls.len()),
            (1, 3, 2, 2)
        );
        assert_eq!(phases.at(3).unwrap().start, 1);
    }
    #[test]
    fn readable_thoughts_join_tools_and_prose_starts_another_phase() {
        let phases = build(&[
            row("user", "Check"),
            tool(),
            row("reasoning", "Check paths"),
            tool(),
            row("assistant", "Now test"),
            tool(),
        ]);
        let group = phases.at(2).unwrap();
        assert_eq!(
            (
                group.start,
                group.end,
                group.last,
                group.steps,
                group.calls.len()
            ),
            (1, 4, 3, 3, 2)
        );
        assert_eq!(phases.at(5).unwrap().start, 5);
    }
    #[test]
    fn initial_placeholder_is_replaced_by_prose_but_not_by_tools() {
        let phases = build(&[
            row("user", "Check"),
            row("reasoning", "Plan"),
            row("assistant", "Inspecting"),
            tool(),
        ]);
        assert!(phases.hidden.contains(&1));
        assert!(phases.at(1).is_none());
        let phases = build(&[row("user", "Check"), row("reasoning", "Plan"), tool()]);
        assert_eq!(phases.at(1).unwrap().steps, 2);
    }

    #[test]
    fn incremental_groups_preserve_source_ids_and_ignore_older_payloads() {
        let phases = collect(
            &[
                row("user", "Earlier"),
                tool(),
                row("assistant", "Done"),
                row("user", "Current"),
                row("assistant", "Checking"),
                row("reasoning", "Compare paths"),
                tool(),
            ],
            5,
            false,
        );
        assert!(phases.at(1).is_none());
        let group = phases.at(5).unwrap();
        assert_eq!((group.start, group.last, group.steps), (5, 6, 2));
        assert!(!phases.hidden.contains(&5));
    }

    #[test]
    fn folded_calls_skip_summaries_and_keep_failed_agents_and_visible_groups() {
        let mut completed = ToolCall::new("read", "Read");
        completed.state = ToolState::Completed;
        completed.output = r#"{"state":"failed","panel":{"kind":"agent"}}"#.repeat(1024);
        let mut agent = ToolCall::new("agent", "Agent");
        agent.state = ToolState::Failed;
        agent.input = r#"{"description":"Review the change"}"#.into();
        let rows = [
            row("user", "Check"),
            row("tool", &serde_json::to_string_pretty(&completed).unwrap()),
            row("tool", &serde_json::to_string_pretty(&agent).unwrap()),
            row("assistant", "Done"),
            tool(),
        ];
        let phases = collect_visible(&rows, 0, false, |at| at < 3);
        assert!(phases.at(1).is_none());
        assert!(phases.pinned.contains(&2));
        assert_eq!(phases.at(4).unwrap().calls.len(), 1);
    }

    #[test]
    fn folded_live_agents_keep_their_standalone_rows() {
        for state in [
            ToolState::Running,
            ToolState::Completed,
            ToolState::Cancelled,
        ] {
            let mut agent = ToolCall::new("agent", "Agent");
            agent.state = state;
            let rows = [
                row("user", "Check"),
                row("tool", &serde_json::to_string(&agent).unwrap()),
                row("assistant", "A reply"),
            ];
            let phases = collect_visible(&rows, 0, true, |_| true);
            assert!(phases.pinned.contains(&1));
            assert!(phases.at(1).is_none());
        }
    }
}
