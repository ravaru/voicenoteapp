use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("voicenote_test_{name}_{now}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn test_job(id: &str, filename: &str) -> Job {
    Job {
        id: id.to_string(),
        filename: filename.to_string(),
        status: "queued".to_string(),
        progress: 0.0,
        stage: "import".to_string(),
        logs: Vec::new(),
        created_at: "0".to_string(),
        audio_path: String::new(),
        transcript_txt_path: String::new(),
        transcript_json_path: String::new(),
        transcript_srt_path: String::new(),
        md_preview: None,
        summary_status: None,
        summary_model: None,
        summary_error: None,
        summary_md: None,
        exported_to_obsidian: false,
    }
}

#[test]
fn log_buffer_is_bounded() {
    let mut job = test_job("job_1", "audio.m4a");
    for idx in 0..2100 {
        push_log(&mut job, &format!("line {idx}"));
    }
    assert_eq!(job.logs.len(), 2000);
    assert_eq!(job.logs.first().cloned(), Some("line 100".to_string()));
    assert_eq!(job.logs.last().cloned(), Some("line 2099".to_string()));
}

#[test]
fn index_persistence_roundtrip() {
    let dir = temp_dir("index_roundtrip");
    let path = dir.join("index.json");
    let index = JobIndex {
        jobs: vec![test_job("job_a", "a.m4a"), test_job("job_b", "b.m4a")],
    };
    save_index_to_disk(&path, &index).expect("save index");
    let loaded = load_index_from_disk(&path).expect("load index");
    assert_eq!(loaded.jobs.len(), 2);
    assert_eq!(loaded.jobs[0].id, "job_a");
    assert_eq!(loaded.jobs[1].id, "job_b");
}

#[test]
fn segments_load_roundtrip() {
    let dir = temp_dir("segments_roundtrip");
    let segments_path = dir.join("segments.json");
    let raw = r#"[{"start":0.0,"end":1.5,"text":"One"},{"start":1.6,"end":3.2,"text":"Two"}]"#;
    fs::write(&segments_path, raw).expect("write segments.json");
    let contents = fs::read_to_string(&segments_path).expect("read segments.json");
    let segments: Vec<Segment> = serde_json::from_str(&contents).expect("parse segments.json");
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].text, "One");
    assert_eq!(segments[1].text, "Two");
}
