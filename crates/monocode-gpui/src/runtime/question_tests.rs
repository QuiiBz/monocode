use super::*;
use monocode_core::agent::Decision;
use std::os::unix::fs::PermissionsExt;

fn answer_transcript(asynchronous: bool) {
    let directory = std::env::temp_dir().join(format!(
        "monocode-question-{}",
        monocode_core::mono::new_id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let binary = directory.join("mock-provider");
    let script = r#"#!/usr/bin/env python3
import json, os, sys, time
def send(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    value = json.loads(line)
    method = value.get('method')
    if method == 'initialize':
        send({'id': value['id'], 'result': {}})
    elif method == 'thread/start':
        send({'id': value['id'], 'result': {'thread': {'id': 'thread'}}})
    elif method == 'turn/start':
        send({'id': value['id'], 'result': {'turn': {'id': 'turn'}}})
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'response', 'delta': 'Before'}})
        if ASYNCHRONOUS:
            send({'method': 'item/started', 'params': {'item': {'id': 'question', 'type': 'agentMessage', 'delivery': 'async', 'questions': [{'title': 'Skip?'}, {'title': 'Choose', 'options': ['B', 'C']}, {'title': 'Details?'}]}}})
        else:
            send({'id': 'question', 'method': 'item/tool/requestUserInput', 'params': {'questions': [{'id': 'skip', 'question': 'Skip?'}, {'id': 'choose', 'question': 'Choose', 'options': [{'label': 'B'}, {'label': 'C'}]}, {'id': 'details', 'question': 'Details?'}]}})
    elif method == 'turn/steer' or value.get('id') == 'question':
        custom = 'Custom 🙂\n' * 400
        if method == 'turn/steer':
            assert value['params']['input'][0]['text'] == 'Choose\nB\nC\n\nDetails?\n' + custom
        else:
            assert value['result']['answers']['details']['answers'] == [custom]
        if method == 'turn/steer':
            send({'id': value['id'], 'result': {}})
        deadline = time.monotonic() + 5
        while not os.path.exists('answer-observed'):
            assert time.monotonic() < deadline, 'Answer was not published'
            time.sleep(0.01)
        send({'method': 'item/agentMessage/delta', 'params': {'itemId': 'response', 'delta': 'After'}})
        send({'method': 'turn/completed', 'params': {'turn': {'status': 'completed'}}})
        break
"#;
    std::fs::write(
        &binary,
        script.replace("ASYNCHRONOUS", if asynchronous { "True" } else { "False" }),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let store = Store::open_monocode(&directory.join("sessions.db")).unwrap();
    let summary = store.create("codex", directory.to_str().unwrap()).unwrap();
    let session_id = summary.id;
    let store = Arc::new(Mutex::new(store));
    let worker_store = store.clone();
    let (updates, receive) = async_channel::unbounded();
    let (controls, control_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        run_turn_using(
            TurnInput {
                summary,
                prompt: "Start".into(),
                settings: Settings {
                    text_only: true,
                    turn_model: Some(monocode_core::agent::TurnInfo {
                        provider: "codex".into(),
                        model: "Question Model".into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            },
            control_rx,
            worker_store,
            Arc::new(AtomicBool::new(false)),
            updates,
            |_| Some(binary),
        );
    });
    let typed_answer = "Custom 🙂\n".repeat(400);
    let expected_answer = format!("Choose\nB\nC\n\nDetails?\n{typed_answer}");
    let mut transcript = Vec::new();
    let mut answer_seen = false;
    loop {
        let update = receive.recv_blocking().unwrap();
        transcript.truncate(update.start);
        transcript.extend(update.rows);
        for question in update.questions {
            controls
                .send(Control {
                    id: question.id,
                    decision: Decision::Answer(vec![
                        vec![],
                        vec!["B".into(), "C".into()],
                        vec![typed_answer.clone()],
                    ]),
                })
                .unwrap();
        }
        if !answer_seen
            && transcript
                .iter()
                .any(|row| row.role == "user" && row.text.starts_with("Choose\n"))
        {
            assert!(!update.finished);
            assert!(!transcript.iter().any(|row| row.text == "After"));
            assert_eq!(
                locked(&store).unwrap().load(session_id).unwrap().rows,
                transcript
            );
            answer_seen = true;
            std::fs::write(directory.join("answer-observed"), "ready").unwrap();
        }
        if update.finished {
            assert_eq!(update.status, "Ready");
            break;
        }
    }
    worker.join().unwrap();
    assert!(answer_seen);
    let saved = locked(&store).unwrap().load(session_id).unwrap();
    assert_eq!(saved.rows, transcript);
    let answer_rows: Vec<_> = saved
        .rows
        .iter()
        .filter(|row| row.role == "user" && row.text != "Start")
        .collect();
    assert_eq!(
        answer_rows
            .iter()
            .map(|row| row.text.as_str())
            .collect::<String>(),
        expected_answer
    );
    assert_eq!(answer_rows.iter().filter(|row| row.first).count(), 1);
    assert!(answer_rows[0].sent_at.is_some());
    assert_eq!(saved.rows.last().unwrap().role, "assistant");
    assert_eq!(saved.rows.last().unwrap().text, "After");
    let settings = locked(&store)
        .unwrap()
        .agent_settings(session_id)
        .unwrap()
        .unwrap();
    let answer_turn = &settings.turns[&(saved.rows.len() - 1)];
    assert_eq!(answer_turn.joined_turn, Some(1));
    assert_eq!(answer_turn.model, "Question Model");
    drop(store);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn structured_answers_are_published_and_saved_before_more_output() {
    answer_transcript(false);
}

#[test]
fn asynchronous_answers_are_published_and_saved_before_more_output() {
    answer_transcript(true);
}
