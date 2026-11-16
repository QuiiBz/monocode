use crate::tool::{tasks::Panel, ToolCall};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fmt,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone)]
pub struct RowText(Text);

#[derive(Clone)]
enum Text {
    Plain(String),
    Tool(Arc<ToolText>),
}

struct ToolText {
    summary: String,
    call: Mutex<Option<Arc<ToolCall>>>,
    encoded: OnceLock<String>,
    bytes: usize,
}

impl RowText {
    pub(crate) fn tool(mut call: ToolCall) -> Result<Self, String> {
        let output = std::mem::take(&mut call.output);
        let details = match &mut call.panel {
            Some(Panel::Agent(run)) => run
                .steps
                .iter_mut()
                .map(|step| std::mem::take(&mut step.detail))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let summary = serde_json::to_string(&call).map_err(|error| error.to_string())?;
        if output.is_empty() && details.iter().all(String::is_empty) {
            return Ok(summary.into());
        }
        let bytes = summary.capacity().saturating_mul(2)
            + output.capacity()
            + details.iter().map(String::capacity).sum::<usize>();
        call.output = output;
        if let Some(Panel::Agent(run)) = &mut call.panel {
            for (step, detail) in run.steps.iter_mut().zip(details) {
                step.detail = detail;
            }
        }
        Ok(Self(Text::Tool(Arc::new(ToolText {
            summary,
            call: Mutex::new(Some(Arc::new(call))),
            encoded: OnceLock::new(),
            bytes,
        }))))
    }

    pub fn display_text(&self) -> &str {
        match &self.0 {
            Text::Plain(text) => text,
            Text::Tool(tool) => &tool.summary,
        }
    }

    pub fn tool_details(&self) -> Option<Arc<ToolCall>> {
        let Text::Tool(tool) = &self.0 else {
            return None;
        };
        if let Some(call) = tool.call.lock().unwrap().as_ref() {
            return Some(call.clone());
        }
        Some(Arc::new(
            serde_json::from_str(self.as_str()).expect("Saved tool JSON remains valid"),
        ))
    }

    pub fn is_deferred(&self) -> bool {
        matches!(self.0, Text::Tool(_))
    }

    pub fn is_materialized(&self) -> bool {
        match &self.0 {
            Text::Plain(_) => true,
            Text::Tool(tool) => tool.encoded.get().is_some(),
        }
    }

    pub fn same_storage(&self, other: &Self) -> bool {
        matches!((&self.0, &other.0), (Text::Tool(left), Text::Tool(right)) if Arc::ptr_eq(left, right))
    }

    pub fn capacity(&self) -> usize {
        match &self.0 {
            Text::Plain(text) => text.capacity(),
            Text::Tool(tool) => tool
                .encoded
                .get()
                .map_or(tool.bytes, |text| text.capacity() + tool.summary.capacity()),
        }
    }
}

impl Default for RowText {
    fn default() -> Self {
        String::new().into()
    }
}

impl From<String> for RowText {
    fn from(text: String) -> Self {
        Self(Text::Plain(text))
    }
}

impl From<&str> for RowText {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl From<RowText> for String {
    fn from(text: RowText) -> Self {
        match text.0 {
            Text::Plain(text) => text,
            Text::Tool(_) => text.as_str().to_owned(),
        }
    }
}

impl Deref for RowText {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        match &self.0 {
            Text::Plain(text) => text,
            Text::Tool(tool) => tool.encoded.get_or_init(|| {
                let call = tool.call.lock().unwrap().take().unwrap();
                serde_json::to_string(&*call).expect("Tool calls serialize to JSON")
            }),
        }
    }
}

impl DerefMut for RowText {
    fn deref_mut(&mut self) -> &mut Self::Target {
        if matches!(self.0, Text::Tool(_)) {
            *self = self.as_str().to_owned().into();
        }
        let Text::Plain(text) = &mut self.0 else {
            unreachable!()
        };
        text
    }
}

impl PartialEq for RowText {
    fn eq(&self, other: &Self) -> bool {
        self.same_storage(other) || self.as_str() == other.as_str()
    }
}

impl Eq for RowText {}

impl PartialEq<str> for RowText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for RowText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for RowText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl fmt::Display for RowText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(formatter)
    }
}

impl fmt::Debug for RowText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display_text().fmt(formatter)
    }
}

impl Serialize for RowText {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RowText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::tasks::{AgentRun, AgentStep};

    #[test]
    fn tool_summaries_keep_outputs_out_of_text_until_requested() {
        let mut call = ToolCall::new("agent", "Agent");
        call.output = "Final report 🙂".repeat(4096);
        call.panel = Some(Panel::Agent(AgentRun {
            steps: vec![AgentStep {
                id: "read".into(),
                kind: "tool".into(),
                text: "Read source".into(),
                detail: "Full nested output".repeat(4096),
                ..Default::default()
            }],
            ..Default::default()
        }));
        let expected = serde_json::to_string(&call).unwrap();
        let text = RowText::tool(call.clone()).unwrap();
        let snapshot = text.clone();
        let summary: ToolCall = serde_json::from_str(text.display_text()).unwrap();
        assert!(summary.output.is_empty());
        let Some(Panel::Agent(run)) = summary.panel else {
            panic!()
        };
        assert_eq!(run.steps[0].text, "Read source");
        assert!(run.steps[0].detail.is_empty());
        assert!(text.same_storage(&snapshot));
        assert_eq!(*text.tool_details().unwrap(), call);
        assert!(!text.is_materialized());
        assert_eq!(text.as_str(), expected);
        assert!(snapshot.is_materialized());
        assert_eq!(*snapshot.tool_details().unwrap(), call);
        assert_eq!(serde_json::to_value(&text).unwrap(), expected);
    }

    #[test]
    fn modifying_a_deferred_clone_keeps_the_original_and_wire_format() {
        let mut call = ToolCall::new("read", "Read");
        call.output = "Saved output".into();
        let text = RowText::tool(call).unwrap();
        let expected = text.as_str().to_owned();
        let mut changed = text.clone();
        changed.push_str(" changed");
        assert_eq!(text.as_str(), expected);
        assert_eq!(changed.as_str(), format!("{expected} changed"));
        assert!(!changed.is_deferred());
        let loaded: RowText = serde_json::from_str(&serde_json::to_string(&text).unwrap()).unwrap();
        assert_eq!(loaded, text);
        assert!(!loaded.is_deferred());
    }
}
