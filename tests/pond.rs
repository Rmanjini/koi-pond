use git_koi::behavior::Behavior;
use git_koi::coat::KoiBody;
use git_koi::collect::{CollectionError, GraphqlClient};
use git_koi::history::LanguageColor;
use git_koi::pond::{PondKoi, collect_pond, render_pond_gif};
use serde_json::{Value, json};

/// Answers every query with one page of three repositories.
struct OnePage;

impl GraphqlClient for OnePage {
    fn run_query(
        &self,
        _query: &str,
        _variables: &[(&str, &str)],
    ) -> Result<Value, CollectionError> {
        let commits = |count: u64| json!({ "target": { "history": { "totalCount": count } } });
        Ok(json!({ "user": { "repositories": {
            "nodes": [
                { "databaseId": 1, "primaryLanguage": null, "defaultBranchRef": null },
                { "databaseId": 2, "primaryLanguage": { "name": "Rust", "color": "#dea584" }, "defaultBranchRef": commits(40) },
                { "databaseId": 3, "primaryLanguage": { "name": "Go", "color": "#00ADD8" }, "defaultBranchRef": commits(7) }
            ],
            "pageInfo": { "hasNextPage": false, "endCursor": null }
        } } }))
    }
}

#[test]
fn pond_is_busiest_first_and_empty_repositories_have_no_commits() {
    let pond = collect_pond(&OnePage, "octocat").unwrap();
    assert_eq!(
        pond.iter()
            .map(|koi| (koi.repository_id, koi.commit_count))
            .collect::<Vec<_>>(),
        [(2, 40), (3, 7), (1, 0)]
    );
    assert_eq!(
        pond[0].language,
        Some(("Rust".to_owned(), Some(LanguageColor([0xde, 0xa5, 0x84]))))
    );
}

#[test]
fn pond_gif_loops_one_cruise_at_the_pond_size() {
    let pond = [
        PondKoi {
            repository_id: 2,
            language: Some(("Rust".to_owned(), Some(LanguageColor([0xde, 0xa5, 0x84])))),
            commit_count: 40,
        },
        PondKoi {
            repository_id: 3,
            language: None,
            commit_count: 1,
        },
    ];
    let gif_bytes = render_pond_gif(&pond, KoiBody::White, Behavior::Cruise).unwrap();
    let mut decoder = gif::DecodeOptions::new()
        .read_info(gif_bytes.as_slice())
        .unwrap();
    // Two koi sit side by side: two 160-pixel cells and a 32-pixel margin on each side.
    assert_eq!((decoder.width(), decoder.height()), (384, 224));
    let mut frame_count = 0;
    while decoder.read_next_frame().unwrap().is_some() {
        frame_count += 1;
    }
    assert_eq!(frame_count, 120);
    assert_eq!(
        render_pond_gif(&pond, KoiBody::White, Behavior::Cruise).unwrap(),
        gif_bytes,
        "deterministic"
    );
}

#[test]
fn feeding_pond_loops_the_feeding_behavior() {
    let pond = [PondKoi {
        repository_id: 5,
        language: None,
        commit_count: 3,
    }];
    let gif_bytes = render_pond_gif(&pond, KoiBody::White, Behavior::SurfaceFeeding).unwrap();
    let mut decoder = gif::DecodeOptions::new()
        .read_info(gif_bytes.as_slice())
        .unwrap();
    let mut frame_count = 0;
    while decoder.read_next_frame().unwrap().is_some() {
        frame_count += 1;
    }
    assert_eq!(frame_count, 90);
}
