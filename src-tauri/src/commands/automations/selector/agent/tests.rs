use serde_json::json;

use super::*;

fn read(result: Value) -> Result<AgentSelection, String> {
    read_result(Some(&result.to_string()))
}

#[test]
fn the_result_lists_the_candidates_and_the_cursor() {
    let result = json!({
        "items": [
            { "id": "ENG-1", "version": "2026-11-01", "title": "First", "data": "{\"team\":\"ENG\"}" },
            { "id": "ENG-2", "version": "v1", "title": null, "data": "" },
            { "id": "ENG-3", "version": "v1", "title": null, "data": { "kept": true } }
        ],
        "cursor": "page-2"
    });

    let selection = read(result.clone()).unwrap();

    assert_eq!(selection.result, result);
    let SelectorOutput::Items { items, cursor } = selection.output else {
        panic!("expected items");
    };
    assert_eq!(cursor, Some(json!("page-2")));
    assert_eq!(
        items
            .iter()
            .map(|item| (item.id.as_str(), item.title.as_deref(), json!(item.data)))
            .collect::<Vec<_>>(),
        [
            ("ENG-1", Some("First"), json!({ "team": "ENG" })),
            ("ENG-2", None, json!({})),
            ("ENG-3", None, json!({ "kept": true })),
        ]
    );
}

#[test]
fn nothing_to_select_keeps_the_cursor() {
    let selection = read_result(Some(" {\"items\":[],\"cursor\":null}\n")).unwrap();
    assert_eq!(
        selection.output,
        SelectorOutput::Items {
            items: Vec::new(),
            cursor: None
        }
    );
    let untitled = read(json!({ "items": [{ "id": "a", "version": "1" }] })).unwrap();
    assert!(
        matches!(untitled.output, SelectorOutput::Items { items, .. } if items[0].data.is_empty())
    );
}

#[test]
fn invalid_results_say_why() {
    let item = |item: Value| json!({ "items": [item], "cursor": null });
    let errors: Vec<String> = [
        json!([1]),
        json!({ "cursor": null }),
        item(json!({ "version": "1", "title": null, "data": "" })),
        item(json!({ "id": "a", "version": 1, "title": null, "data": "" })),
        item(json!({ "id": "a", "version": "1", "title": 3, "data": "" })),
        item(json!({ "id": "a", "version": "1", "title": null, "data": "[1]" })),
        item(json!({ "id": "a", "version": "1", "title": null, "data": "{" })),
        item(json!({ "id": "a", "version": "1", "title": null, "data": 7 })),
        item(json!({ "id": "a b", "version": "1", "title": null, "data": "" })),
        item(json!({ "id": "a", "version": "", "title": null, "data": "" })),
        json!({ "items": [], "cursor": { "page": 2 } }),
    ]
    .into_iter()
    .map(|result| read(result).err().unwrap())
    .collect();

    assert_eq!(
        errors,
        [
            "the result has no items list",
            "the result has no items list",
            "items[0]: id must be a string",
            "items[0]: version must be a string",
            "items[0]: title must be a string or null",
            "items[0]: data must be the text of a JSON object",
            "items[0]: data must be the text of a JSON object",
            "items[0]: data must be the text of a JSON object",
            "selector item id is invalid: \"a b\"",
            "selector item a has an empty version",
            "cursor must be a string or null",
        ]
    );
    assert_eq!(
        read_result(None).err().as_deref(),
        Some("the agent replied without a result")
    );
    assert_eq!(
        read_result(Some("  ")).err().as_deref(),
        Some("the agent replied without a result")
    );
    assert!(read_result(Some("not json"))
        .err()
        .unwrap()
        .starts_with("the reply is not JSON"));
}

#[test]
fn the_schema_requires_every_field_and_allows_no_other() {
    let schema = result_schema();
    assert_eq!(schema["required"], json!(["items", "cursor"]));
    let item = &schema["properties"]["items"]["items"];
    assert_eq!(item["required"], json!(["id", "version", "title", "data"]));
    assert_eq!(item["additionalProperties"], json!(false));
}
