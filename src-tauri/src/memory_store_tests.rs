//! 仅内存SQLite和临时虚构文件；不打开个人数据库，不发送API或操作窗口。
use super::*;
fn store() -> Store {
    let mut db = Connection::open_in_memory().unwrap();
    crate::chat_store::upgrade_schema(&mut db, 0).unwrap();
    let store = Store::test_database(db);
    store.set("memory_enabled", "true").unwrap();
    store
}
fn source(s: &Store, text: &str) -> i64 {
    s.db.execute("INSERT INTO memory_sources(source_key,kind,user_text,reply,created_at,captured_epoch) VALUES(?1,'chat',?2,'好的',1,?3)",params![id().unwrap(),text,s.memory_epoch().unwrap()]).unwrap();
    s.db.last_insert_rowid()
}
fn claim(s: &mut Store) -> Job {
    s.memory_claim(true, "fixture").unwrap().0.unwrap()
}
fn output(job: &Job, topic: &str, text: &str, summary: &str) -> Distillation {
    Distillation {
        summary: summary.into(),
        facts: vec![Fact {
            topic: topic.into(),
            text: text.into(),
            evidence: vec![Evidence {
                source: job.sources[0].id,
                quote: job.sources[0].user_text.clone(),
            }],
        }],
    }
}
fn save(s: &mut Store, topic: &str, text: &str, summary: &str) -> i64 {
    source(s, text);
    let job = claim(s);
    s.memory_save(&job, output(&job, topic, text, summary), "fixture", &None)
        .unwrap();
    s.memory_entries()
        .unwrap()
        .iter()
        .find(|e| e.kind == "fact" && e.topic == topic)
        .unwrap()
        .id
}
fn count(s: &Store, sql: &str) -> i64 {
    s.db.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn eleventh_fact_is_saved_and_prompt_is_bounded() {
    let mut s = store();
    for n in 0..25 {
        save(
            &mut s,
            &format!("爱好{n}"),
            &format!("我喜欢虚构花朵{n}"),
            "",
        );
    }
    assert_eq!(
        count(&s, "SELECT count(*) FROM memory_entries WHERE kind='fact'"),
        25
    );
    let recall = s.memory_recall("虚构花朵24", None).unwrap();
    assert!(recall.contains("虚构花朵24"));
    assert!(recall.lines().filter(|l| l.starts_with("- ")).count() <= 13);
    assert!(recall.chars().count() < 2600);
}

#[test]
fn edit_preserves_unrelated_pending_and_rejects_late_result() {
    let mut s = store();
    let entry = save(&mut s, "颜色", "我喜欢蓝色", "");
    source(&s, "我喜欢书签");
    source(&s, "我喜欢纸鹤");
    let old = claim(&mut s);
    s.memory_mutate("edit", entry, "我喜欢绿色").unwrap();
    assert!(s
        .memory_save(
            &old,
            output(&old, "书签", "我喜欢书签", "旧结果"),
            "fixture",
            &None
        )
        .is_err());
    let new = claim(&mut s);
    assert_eq!(new.sources.len(), 2);
    s.memory_pause(&old, None).unwrap();
    s.memory_save(
        &new,
        output(&new, "书签", "我喜欢书签", "整理保留下来的待办"),
        "fixture",
        &None,
    )
    .unwrap();
    assert!(s
        .memory_entries()
        .unwrap()
        .iter()
        .any(|e| e.text == "我喜欢绿色" && e.edited && e.evidence.is_empty()));
    assert!(s.memory_jobs().unwrap().iter().any(|j| j
        .result
        .as_deref()
        .is_some_and(|r| r.contains("事实保存1项"))));
}

#[test]
fn source_scope_filters_mixed_batch_and_new_expression_can_be_learned() {
    let mut s = store();
    let entry = save(&mut s, "颜色", "我喜欢蓝色", "蓝色杯垫");
    let old_source = s
        .memory_entries()
        .unwrap()
        .iter()
        .find(|e| e.id == entry)
        .unwrap()
        .source_ids[0];
    s.db.execute(
        "UPDATE memory_sources SET job_id=NULL WHERE id=?",
        [old_source],
    )
    .unwrap();
    let unrelated = source(&s, "我喜欢木制书签");
    let old = claim(&mut s);
    assert_eq!(old.sources.len(), 2);
    s.memory_mutate("delete", entry, "").unwrap();
    let new = claim(&mut s);
    assert_eq!(
        new.sources.iter().map(|s| s.id).collect::<Vec<_>>(),
        vec![unrelated]
    );
    s.memory_save(
        &new,
        output(&new, "书签", "我喜欢木制书签", "书签"),
        "fixture",
        &None,
    )
    .unwrap();
    save(&mut s, "颜色", "我喜欢绿色", "绿色杯垫");
    let recall = s.memory_recall("杯垫颜色", None).unwrap();
    assert!(recall.contains("我喜欢绿色"));
    assert!(!recall.contains("我喜欢蓝色"));
}

#[test]
fn manual_fact_keeps_conflict_candidate_and_archives_conflicting_summary() {
    let mut s = store();
    let entry = save(&mut s, "饮料", "我喜欢红茶", "");
    s.memory_mutate("edit", entry, "我喜欢白茶").unwrap();
    save(&mut s, "饮料", "我喜欢绿茶", "我喜欢绿茶");
    let entries = s.memory_entries().unwrap();
    let candidate = entries.iter().find(|e| e.kind == "candidate").unwrap();
    assert_eq!(candidate.text, "我喜欢绿茶");
    assert!(entries
        .iter()
        .any(|e| e.kind == "summary" && e.text == "我喜欢绿茶" && e.archived));
    assert!(!s
        .memory_recall("绿茶", None)
        .unwrap()
        .contains("我喜欢绿茶"));
    s.memory_mutate("edit", candidate.id, "我喜欢绿茶").unwrap();
    assert!(s.memory_recall("茶", None).unwrap().contains("我喜欢绿茶"));
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_entries WHERE kind='fact' AND quarantined=0"
        ),
        1
    );
}

#[test]
fn newer_fact_preserves_old_version_but_not_old_summary_in_recall() {
    let mut s = store();
    save(&mut s, "颜色", "我喜欢蓝色", "蓝色杯垫");
    save(&mut s, "颜色", "我喜欢绿色", "绿色杯垫");
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_entries WHERE kind='superseded'"
        ),
        1
    );
    let recall = s.memory_recall("杯垫", None).unwrap();
    assert!(!recall.contains("蓝色"));
    assert!(recall.contains("绿色"));
}

#[test]
fn entry_only_deletion_does_not_cascade_or_block_sources() {
    let mut s = store();
    let entry = save(&mut s, "颜色", "我喜欢蓝色", "蓝色杯垫");
    s.memory_mutate("delete_entry", entry, "").unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_entries WHERE kind='summary'"
        ),
        1
    );
    assert_eq!(
        count(&s, "SELECT count(*) FROM memory_sources WHERE blocked=1"),
        0
    );
}

#[test]
fn original_deletion_removes_only_selected_request_and_attempts() {
    let mut s = store();
    s.db.execute_batch("INSERT INTO conversations VALUES('conversation','虚构',1);
        INSERT INTO messages VALUES(1,'conversation','selected','user','user','我喜欢蓝色','complete',1,'fixture',NULL,NULL);
        INSERT INTO messages VALUES(2,'conversation','selected','assistant','user','好的','complete',1,'fixture',NULL,NULL);
        INSERT INTO messages VALUES(3,'conversation','other','user','user','保留问答','complete',1,'fixture',NULL,NULL);
        INSERT INTO messages VALUES(4,'conversation','other','assistant','user','保留回复','complete',1,'fixture',NULL,NULL);
        INSERT INTO chat_attempts VALUES('a','selected','fixture','虚构','fixture','complete','尝试回复',NULL,NULL,1);").unwrap();
    let job = claim(&mut s);
    let out = output(&job, "颜色", "我喜欢蓝色", "蓝色杯垫");
    s.memory_save(&job, out, "fixture", &None).unwrap();
    let entry = s
        .memory_entries()
        .unwrap()
        .iter()
        .find(|e| e.kind == "fact")
        .unwrap()
        .id;
    s.memory_mutate("delete_original", entry, "").unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM messages WHERE request_id='selected'"
        ),
        0
    );
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM chat_attempts WHERE request_id='selected'"
        ),
        0
    );
    assert_eq!(
        count(&s, "SELECT count(*) FROM messages WHERE request_id='other'"),
        2
    );
    assert_eq!(count(&s,"SELECT count(*) FROM memory_sources WHERE source_key='chat:selected' AND blocked=1 AND user_text='' AND reply=''"),1);
    s.memory_capture().unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_sources WHERE source_key='chat:selected'"
        ),
        1
    );
    assert!(!s
        .context("conversation")
        .unwrap()
        .iter()
        .any(|v| v["content"].as_str() == Some("我喜欢蓝色")));
}

#[test]
fn transaction_failure_keeps_original_and_pending_job() {
    let mut s = store();
    let entry = save(&mut s, "颜色", "我喜欢蓝色", "蓝色杯垫");
    source(&s, "我喜欢书签");
    let job = claim(&mut s);
    s.db.execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON memory_entries BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let epoch = s.memory_epoch().unwrap();
    assert!(s.memory_mutate("delete", entry, "").is_err());
    assert_eq!(s.memory_epoch().unwrap(), epoch);
    assert_eq!(
        count(&s, "SELECT count(*) FROM memory_sources WHERE blocked=1"),
        0
    );
    assert!(s.memory_entries().unwrap().iter().any(|e| e.id == entry));
    s.memory_save(
        &job,
        output(&job, "书签", "我喜欢书签", ""),
        "fixture",
        &None,
    )
    .unwrap();
}

#[test]
fn save_is_atomic_and_same_result_cannot_be_replayed() {
    let mut s = store();
    source(&s, "我喜欢花朵");
    let job = claim(&mut s);
    s.db.execute_batch("CREATE TRIGGER fail_save BEFORE INSERT ON memory_entries WHEN NEW.kind='fact' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(s
        .memory_save(
            &job,
            output(&job, "花朵", "我喜欢花朵", "花朵"),
            "fixture",
            &None
        )
        .is_err());
    assert_eq!(count(&s, "SELECT count(*) FROM memory_entries"), 0);
    s.db.execute_batch("DROP TRIGGER fail_save").unwrap();
    s.memory_save(
        &job,
        output(&job, "花朵", "我喜欢花朵", "花朵"),
        "fixture",
        &None,
    )
    .unwrap();
    assert!(s
        .memory_save(
            &job,
            output(&job, "花朵", "我喜欢花朵", "花朵"),
            "fixture",
            &None
        )
        .is_err());
}

#[test]
fn duplicates_archive_and_important_old_event_beats_recent_smalltalk() {
    let mut s = store();
    for n in 0..8 {
        save(
            &mut s,
            &format!("主题{n}"),
            &format!("我喜欢测试花朵{n}"),
            "今天聊了虚构天气",
        );
    }
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_entries WHERE kind='summary' AND archived=0"
        ),
        1
    );
    s.db.execute("INSERT INTO memory_entries(kind,topic,text,source_ids,evidence,created_at,updated_at) VALUES('summary','','去年完成了虚构的登山纪念旅行','[]','[]',0,0)",[]).unwrap();
    let recall = s.memory_recall("登山纪念旅行", None).unwrap();
    assert!(recall.contains("去年完成"));
    assert!(!recall.contains("虚构天气"));
}

#[test]
fn similarity_diversifies_and_recall_growth_is_measured() {
    let s = store();
    let started = std::time::Instant::now();
    let mut vector = vec![0.0; 512];
    vector[0] = 1.0;
    for n in 0..2000 {
        s.db.execute("INSERT INTO memory_entries(kind,topic,text,source_ids,evidence,created_at,updated_at) VALUES('summary','',?1,'[]','[]',?2,?2)",params![format!("虚构闲聊记录第{n}次"),n]).unwrap();
        s.memory_index(
            s.db.last_insert_rowid(),
            &format!("虚构闲聊记录第{n}次"),
            &vector,
        )
        .unwrap();
    }
    let recall_started = std::time::Instant::now();
    let recall = s.memory_recall("闲聊", Some(&vector)).unwrap();
    eprintln!(
        "synthetic 2000 entries: setup={:?}, recall={:?}",
        started.elapsed(),
        recall_started.elapsed()
    );
    assert_eq!(recall.lines().filter(|l| l.starts_with("- ")).count(), 1);
}

#[test]
fn v5_migration_preserves_pending_and_marks_historical_loss() {
    let mut s = store();
    let entry = save(&mut s, "颜色", "我喜欢蓝色", "");
    source(&s, "我喜欢书签");
    let old = claim(&mut s);
    s.memory_mutate("edit", entry, "我喜欢绿色").unwrap();
    s.db.execute_batch(
        "DROP INDEX memory_source_queue; DROP INDEX memory_recall_active;
        ALTER TABLE memory_sources DROP COLUMN blocked;
        ALTER TABLE memory_entries DROP COLUMN archived;
        ALTER TABLE memory_entries DROP COLUMN archive_reason;
        ALTER TABLE memory_jobs DROP COLUMN result;
        INSERT INTO config VALUES('memory_forget_cutoff','999');
        PRAGMA user_version=5;",
    )
    .unwrap();
    migrate_v6(&mut s.db).unwrap();
    assert!(s.config("memory_forget_cutoff").unwrap().is_none());
    assert!(s.memory_entries().unwrap().iter().any(|e| e.id == entry));
    assert_eq!(count(&s,"SELECT count(*) FROM memory_sources WHERE blocked=1"),1);
    assert!(s
        .memory_save(
            &old,
            output(&old, "书签", "我喜欢书签", ""),
            "fixture",
            &None
        )
        .is_err());
    let resumed = claim(&mut s);
    assert_eq!(resumed.sources.len(), 1);
    assert_eq!(resumed.sources[0].user_text, "我喜欢书签");
}

#[test]
fn migration_failure_rolls_back_and_unknown_version_is_untouched() {
    let mut s = store();
    s.db.execute_batch("DROP INDEX memory_recall_active; DROP INDEX memory_source_queue; ALTER TABLE memory_sources DROP COLUMN blocked; ALTER TABLE memory_entries DROP COLUMN archived; PRAGMA user_version=5;").unwrap();
    assert!(migrate_v6(&mut s.db).is_err());
    assert_eq!(count(&s, "PRAGMA user_version"), 5);
    assert!(s.db.prepare("SELECT archived FROM memory_entries").is_err());
    assert!(s.db.prepare("SELECT blocked FROM memory_sources").is_err());
    assert!(crate::chat_store::upgrade_schema(&mut s.db, 999).is_err());
    assert_eq!(count(&s, "PRAGMA user_version"), 5);
}

#[test]
fn clear_removes_all_memory_states_and_source_stubs() {
    let mut s = store();
    let entry = save(&mut s, "颜色", "我喜欢蓝色", "");
    s.memory_mutate("delete", entry, "").unwrap();
    s.clear().unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM memory_sources"), 0);
    assert_eq!(count(&s, "SELECT count(*) FROM memory_jobs"), 0);
}

#[test]
fn schema1_upgrade_preserves_chat_and_does_not_backfill_paid_jobs() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE conversations(id TEXT PRIMARY KEY,title TEXT NOT NULL,created_at INTEGER NOT NULL);
        CREATE TABLE messages(id INTEGER PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),request_id TEXT NOT NULL,role TEXT NOT NULL,source TEXT NOT NULL,text TEXT NOT NULL,status TEXT NOT NULL,created_at INTEGER NOT NULL,provider TEXT NOT NULL,usage TEXT,error TEXT);
        CREATE TABLE config(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE life_events(event_id TEXT PRIMARY KEY,created_at INTEGER NOT NULL);
        CREATE TABLE auto_requests(request_id TEXT PRIMARY KEY,created_at INTEGER NOT NULL);
        INSERT INTO conversations VALUES('fixture','虚构',1);
        INSERT INTO messages VALUES(1,'fixture','request','user','user','保留旧问答','complete',1,'fixture',NULL,NULL);
        INSERT INTO messages VALUES(2,'fixture','request','assistant','user','保留旧回复','complete',1,'fixture',NULL,NULL);
        PRAGMA user_version=1;").unwrap();
    crate::chat_store::upgrade_schema(&mut db, 1).unwrap();
    let mut s = Store::test_database(db);
    assert_eq!(count(&s, "PRAGMA user_version"), 6);
    assert_eq!(count(&s, "SELECT count(*) FROM messages"), 2);
    assert!(!s.memory_enabled().unwrap());
    s.set("memory_enabled", "true").unwrap();
    assert!(s.memory_claim(true, "fixture").unwrap().0.is_none());
}

#[test]
fn real_database_reopen_resumes_sources_but_rejects_previous_attempt() {
    let path = std::env::temp_dir().join(format!("moon-memory-fixture-{}.sqlite3", id().unwrap()));
    let mut db = Connection::open(&path).unwrap();
    crate::chat_store::upgrade_schema(&mut db, 0).unwrap();
    let old = {
        let mut s = Store::test_database(db);
        s.set("memory_enabled", "true").unwrap();
        source(&s, "我喜欢纸鹤");
        claim(&mut s)
    };
    {
        let mut s = Store::test_database(Connection::open(&path).unwrap());
        s.recover_interrupted().unwrap();
        let new = claim(&mut s);
        assert_eq!(new.sources[0].user_text, "我喜欢纸鹤");
        s.memory_pause(&old, None).unwrap();
        assert!(s
            .memory_save(
                &old,
                output(&old, "纸鹤", "我喜欢纸鹤", "旧回包"),
                "fixture",
                &None
            )
            .is_err());
        s.memory_save(
            &new,
            output(&new, "纸鹤", "我喜欢纸鹤", "纸鹤"),
            "fixture",
            &None,
        )
        .unwrap();
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn stale_pending_source_does_not_overwrite_newer_fact_or_return_in_summary() {
    let mut s = store();
    let old_source = source(&s, "我喜欢蓝色");
    s.db.execute("INSERT INTO memory_jobs(id,epoch,status,created_at,route_id) VALUES('old',?1,'queued',0,'other-provider')",[s.memory_epoch().unwrap()]).unwrap();
    s.db.execute(
        "UPDATE memory_sources SET job_id='old' WHERE id=?",
        [old_source],
    )
    .unwrap();
    save(&mut s, "颜色", "我喜欢绿色", "绿色杯垫");
    let old = s.memory_claim(true, "other-provider").unwrap().0.unwrap();
    s.memory_save(
        &old,
        output(&old, "颜色", "我喜欢蓝色", "蓝色杯垫"),
        "other-provider",
        &None,
    )
    .unwrap();
    let recall = s.memory_recall("杯垫颜色", None).unwrap();
    assert!(recall.contains("绿色"));
    assert!(!recall.contains("蓝色"));
    assert!(s
        .memory_entries()
        .unwrap()
        .iter()
        .any(|e| e.kind == "candidate" && e.text == "我喜欢蓝色"));
}

#[test]
fn corrupt_epoch_does_not_start_schema_migration() {
    let mut s = store();
    s.set("memory_epoch", "broken").unwrap();
    assert!(migrate_v6(&mut s.db).is_err());
    assert_eq!(s.config("memory_epoch").unwrap().as_deref(), Some("broken"));
}

#[test]
fn same_source_multiple_facts_delete_together_without_deleting_other_pending_sources() {
    let mut s = store();
    source(&s, "我喜欢蓝色杯垫和木制书签");
    let job = claim(&mut s);
    let mut out = output(&job, "颜色", "我喜欢蓝色杯垫", "杯垫和书签");
    out.facts.push(Fact {
        topic: "书签".into(),
        text: "我喜欢木制书签".into(),
        evidence: out.facts[0].evidence.clone(),
    });
    s.memory_save(&job, out, "fixture", &None).unwrap();
    let entry = s
        .memory_entries()
        .unwrap()
        .iter()
        .find(|e| e.kind == "fact")
        .unwrap()
        .id;
    source(&s, "我喜欢折纸");
    s.memory_mutate("delete", entry, "").unwrap();
    assert!(s.memory_entries().unwrap().is_empty());
    assert_eq!(claim(&mut s).sources[0].user_text, "我喜欢折纸");
}

#[test]
fn failed_job_keeps_sources_and_retry_requeues_it() {
    let mut s = store();
    source(&s, "我喜欢折纸");
    let job = claim(&mut s);
    s.memory_fail(&job.id, "虚构网络失败", "fixture", &None)
        .unwrap();
    let retry = claim(&mut s);
    s.memory_fail(&retry.id, "虚构网络失败", "fixture", &None)
        .unwrap();
    assert_eq!(s.memory_jobs().unwrap()[0].status, "failed");
    assert!(s.memory_claim(true, "fixture").unwrap().0.is_none());
    s.memory_mutate("retry", 0, "").unwrap();
    let resumed = claim(&mut s);
    assert_eq!(resumed.sources[0].user_text, "我喜欢折纸");
    s.memory_save(
        &resumed,
        output(&resumed, "折纸", "我喜欢折纸", ""),
        "fixture",
        &None,
    )
    .unwrap();
}

#[test]
fn adopting_an_old_version_archives_summary_of_replaced_current_fact() {
    let mut s = store();
    let old = save(&mut s, "颜色", "我喜欢蓝色", "蓝色杯垫");
    save(&mut s, "颜色", "我喜欢绿色", "绿色杯垫");
    s.memory_mutate("edit", old, "我喜欢蓝色").unwrap();
    let recall = s.memory_recall("杯垫", None).unwrap();
    assert!(recall.contains("蓝色"));
    assert!(!recall.contains("绿色"));
}
