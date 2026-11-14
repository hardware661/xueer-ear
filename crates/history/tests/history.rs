use xueer_ear_history::{Entry, History, Settings};

const DAY_MS: u64 = 86_400_000;

fn entry(at_ms: u64, text: &str) -> Entry {
    Entry {
        at_ms,
        text: text.into(),
        app: "notepad.exe".into(),
    }
}

#[test]
fn append_then_read_newest_first() {
    let dir = tempfile::tempdir().unwrap();
    let h = History::new(dir.path().join("history.jsonl"), Settings::default());
    h.append(&entry(1, "第一句")).unwrap();
    h.append(&entry(2, "第二句")).unwrap();
    let got = h.recent(10).unwrap();
    assert_eq!(
        got.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
        ["第二句", "第一句"]
    );
    assert_eq!(h.recent(1).unwrap().len(), 1);
}

#[test]
fn default_keeps_seven_days() {
    assert_eq!(Settings::default().keep_days, 7);
    assert!(Settings::default().enabled);
}

#[test]
fn prune_drops_entries_older_than_keep_days() {
    let dir = tempfile::tempdir().unwrap();
    let h = History::new(
        dir.path().join("history.jsonl"),
        Settings {
            enabled: true,
            keep_days: 7,
        },
    );
    let now = 100 * DAY_MS;
    h.append(&entry(now - 8 * DAY_MS, "太旧")).unwrap();
    h.append(&entry(now - 6 * DAY_MS, "还在")).unwrap();
    h.append(&entry(now, "今天")).unwrap();
    assert_eq!(h.prune(now).unwrap(), 1);
    let texts: Vec<_> = h.recent(10).unwrap().into_iter().map(|e| e.text).collect();
    assert_eq!(texts, ["今天", "还在"]);
}

#[test]
fn disabled_history_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let h = History::new(
        path.clone(),
        Settings {
            enabled: false,
            keep_days: 7,
        },
    );
    h.append(&entry(1, "不该记")).unwrap();
    assert!(!path.exists());
    assert!(h.recent(10).unwrap().is_empty());
}

#[test]
fn missing_file_is_empty_and_bad_lines_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let h = History::new(path.clone(), Settings::default());
    assert!(h.recent(10).unwrap().is_empty());
    std::fs::write(
        &path,
        "not json\n{\"at_ms\":5,\"text\":\"好的\",\"app\":\"\"}\n{\"at_ms\":6,\"te",
    )
    .unwrap();
    let got = h.recent(10).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].text, "好的");
}

#[test]
fn text_with_newlines_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let h = History::new(dir.path().join("history.jsonl"), Settings::default());
    h.append(&entry(1, "第一行\n第二行")).unwrap();
    assert_eq!(h.recent(1).unwrap()[0].text, "第一行\n第二行");
}
