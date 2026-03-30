use malt_term::completion::{
    Candidate, CompletionCoordinator, CompletionKind, CompletionSource, PathCompletionSource,
    common_prefix,
};

/// A mock source that returns fixed candidates.
struct MockSource {
    candidates: Vec<Candidate>,
}

impl CompletionSource for MockSource {
    fn complete(&self, _line: &str, _cursor: usize) -> Vec<Candidate> {
        self.candidates.clone()
    }
}

#[test]
fn mock_source_returns_candidates() {
    let source = MockSource {
        candidates: vec![
            Candidate {
                value: "foo".into(),
                display: "foo".into(),
                kind: CompletionKind::Command,
            },
            Candidate {
                value: "bar".into(),
                display: "bar".into(),
                kind: CompletionKind::Command,
            },
        ],
    };

    let mut coord = CompletionCoordinator::new();
    coord.add_source(Box::new(source));

    let results = coord.complete("f", 1);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].value, "foo");
    assert_eq!(results[1].value, "bar");
}

#[test]
fn common_prefix_calculation() {
    let candidates = vec![
        Candidate {
            value: "foo".into(),
            display: "foo".into(),
            kind: CompletionKind::Command,
        },
        Candidate {
            value: "foobar".into(),
            display: "foobar".into(),
            kind: CompletionKind::Command,
        },
        Candidate {
            value: "foobaz".into(),
            display: "foobaz".into(),
            kind: CompletionKind::Command,
        },
    ];
    assert_eq!(common_prefix(&candidates), "foo");
}

#[test]
fn common_prefix_empty_list() {
    assert_eq!(common_prefix(&[]), "");
}

#[test]
fn common_prefix_no_shared() {
    let candidates = vec![
        Candidate {
            value: "abc".into(),
            display: "abc".into(),
            kind: CompletionKind::Command,
        },
        Candidate {
            value: "xyz".into(),
            display: "xyz".into(),
            kind: CompletionKind::Command,
        },
    ];
    assert_eq!(common_prefix(&candidates), "");
}

#[test]
fn path_completion_finds_files() {
    let dir = tempfile::TempDir::new().expect("create tempdir");
    let file_path = dir.path().join("testfile.txt");
    std::fs::write(&file_path, b"hello").expect("write testfile");
    let sub = dir.path().join("subdir");
    std::fs::create_dir(&sub).expect("create subdir");

    let source = PathCompletionSource;
    let prefix = format!("ls {}/", dir.path().display());
    let cursor = prefix.len();
    let results = source.complete(&prefix, cursor);

    assert!(
        results.iter().any(|c| c.value.contains("testfile.txt")),
        "should find testfile.txt, got: {:?}",
        results
    );
    assert!(
        results.iter().any(|c| c.kind == CompletionKind::Directory),
        "should find a directory, got: {:?}",
        results
    );
}

#[test]
fn empty_returns_no_candidates() {
    let source = PathCompletionSource;
    let results = source.complete("", 0);
    assert!(results.is_empty());
}
