//! Saved reads of Claude transcripts that held no launch (migration 33).
use xt_store::{
    Store,
    transcript_read::{SavedTranscriptRead, TranscriptReader},
};

fn prompt_read(path: &str) -> SavedTranscriptRead {
    SavedTranscriptRead {
        reader: TranscriptReader::Prompt,
        path: path.to_owned(),
        owner_native_session_id: "0d000000-0000-4000-8000-0000000000ca".to_owned(),
        reader_version: 1,
        max_line: 16 << 20,
        rules_digest: [7; 32],
        device: 1,
        inode: 2,
        read_to: 4_096,
        mtime_ns: 11,
        ctime_ns: 12,
        fingerprint: [9; 32],
        untimed: true,
        refused_ms: vec![1_790_668_716_333, -5],
    }
}

fn bash_read(path: &str) -> SavedTranscriptRead {
    SavedTranscriptRead {
        reader: TranscriptReader::Bash,
        untimed: false,
        refused_ms: Vec::new(),
        ..prompt_read(path)
    }
}

#[test]
fn a_saved_read_round_trips_per_reader_and_is_replaced_or_forgotten() {
    let mut store = Store::open_in_memory().unwrap();
    let path = "/home/.claude/projects/-w/0d000000-0000-4000-8000-0000000000ca.jsonl";
    assert_eq!(
        store
            .saved_transcript_read(TranscriptReader::Prompt, path)
            .unwrap(),
        None
    );
    store
        .save_transcript_reads(&[prompt_read(path), bash_read(path)], &[], 10)
        .unwrap();
    assert_eq!(
        store
            .saved_transcript_read(TranscriptReader::Prompt, path)
            .unwrap(),
        Some(prompt_read(path))
    );
    assert_eq!(
        store
            .saved_transcript_read(TranscriptReader::Bash, path)
            .unwrap(),
        Some(bash_read(path))
    );
    // A later read replaces the earlier one.
    let later = SavedTranscriptRead {
        read_to: 8_192,
        refused_ms: Vec::new(),
        untimed: false,
        ..prompt_read(path)
    };
    store
        .save_transcript_reads(std::slice::from_ref(&later), &[], 20)
        .unwrap();
    assert_eq!(
        store
            .saved_transcript_read(TranscriptReader::Prompt, path)
            .unwrap(),
        Some(later)
    );
    // Forgetting one reader's read keeps the other's.
    store
        .save_transcript_reads(&[], &[(TranscriptReader::Prompt, path.to_owned())], 30)
        .unwrap();
    assert_eq!(
        store
            .saved_transcript_read(TranscriptReader::Prompt, path)
            .unwrap(),
        None
    );
    assert!(
        store
            .saved_transcript_read(TranscriptReader::Bash, path)
            .unwrap()
            .is_some()
    );
}

#[test]
fn the_table_refuses_a_row_it_does_not_describe() {
    let mut store = Store::open_in_memory().unwrap();
    let path = "/home/.claude/projects/-w/x.jsonl";
    for bad in [
        // Only `prompt` keeps what is not a launch.
        SavedTranscriptRead {
            refused_ms: vec![1],
            ..bash_read(path)
        },
        SavedTranscriptRead {
            untimed: true,
            ..bash_read(path)
        },
        // A path that is not absolute, an empty owner.
        prompt_read("relative.jsonl"),
        SavedTranscriptRead {
            owner_native_session_id: " ".to_owned(),
            ..prompt_read(path)
        },
        SavedTranscriptRead {
            reader_version: 0,
            ..prompt_read(path)
        },
    ] {
        assert!(
            store
                .save_transcript_reads(std::slice::from_ref(&bad), &[], 1)
                .is_err(),
            "{bad:?}"
        );
    }
    // A refused batch writes nothing, not even its valid rows.
    assert!(
        store
            .save_transcript_reads(&[prompt_read(path), prompt_read("bad")], &[], 1)
            .is_err()
    );
    assert_eq!(
        store
            .saved_transcript_read(TranscriptReader::Prompt, path)
            .unwrap(),
        None
    );
}
