use chatgpt_thread_exporter::ArtifactResolver;
use serde_json::{Value, json};

fn api_artifact(index: usize) -> Value {
    json!({
        "id": format!("artifact-{index}"),
        "direction": "received",
        "message_id": format!("message-{index}"),
        "pointer": "file-example",
        "pointer_aliases": [],
        "pointer_kind": "file_id",
        "suggested_name": "example.bin",
        "mime_type": null,
        "size_bytes": null,
        "relative_path": format!("received/{index}.bin"),
        "source": "test",
        "resolution": [{"kind": "api", "path": format!("/backend-api/files/download/file-{index}")}]
    })
}

fn task(resolver: &mut ArtifactResolver) -> Result<Value, Box<dyn std::error::Error>> {
    let json = resolver.next_request()?.ok_or("expected a request")?;
    Ok(serde_json::from_str(&json)?)
}

#[test]
fn bounds_concurrency_and_refills_a_free_slot() -> Result<(), Box<dyn std::error::Error>> {
    let artifacts: Vec<_> = (0..6).map(api_artifact).collect();
    let mut resolver = ArtifactResolver::new(&serde_json::to_string(&artifacts)?)?;
    assert_eq!(resolver.worker_count(), 4);
    for index in 0..4 {
        assert_eq!(task(&mut resolver)?["index"], index);
    }
    assert!(resolver.next_request()?.is_none());
    resolver.complete(
        1,
        r#"{"value":{"download_url":"https://files.oaiusercontent.com/example.bin"}}"#,
    )?;
    assert_eq!(task(&mut resolver)?["index"], 4);
    assert!(resolver.next_request()?.is_none());
    Ok(())
}

#[test]
fn keeps_results_in_plan_order() -> Result<(), Box<dyn std::error::Error>> {
    let inputs: Vec<_> = (0..3).map(api_artifact).collect();
    let mut resolver = ArtifactResolver::new(&serde_json::to_string(&inputs)?)?;
    for _ in 0..3 {
        task(&mut resolver)?;
    }
    for index in [2, 0, 1] {
        resolver.complete(
            index,
            r#"{"value":{"download_url":"https://files.oaiusercontent.com/example.bin"}}"#,
        )?;
    }
    assert!(resolver.next_request()?.is_none());
    let results: Vec<Value> = serde_json::from_str(&resolver.finish()?)?;
    let ids: Vec<_> = results
        .iter()
        .map(|result| result["artifact_id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [Some("artifact-0"), Some("artifact-1"), Some("artifact-2")]
    );
    Ok(())
}

#[test]
fn falls_back_after_failure_and_rejects_untrusted_responses()
-> Result<(), Box<dyn std::error::Error>> {
    let mut input = api_artifact(0);
    input["resolution"] = json!([
        {"kind": "api", "path": "/backend-api/files/download/file-example"},
        {"kind": "remote", "url": "https://files.oaiusercontent.com/fallback.bin"}
    ]);
    let mut resolver = ArtifactResolver::new(&json!([input, api_artifact(1)]).to_string())?;
    task(&mut resolver)?;
    task(&mut resolver)?;
    resolver.complete(
        0,
        r#"{"value":{"download_url":"https://attacker.example/file"}}"#,
    )?;
    resolver.complete(1, r#"{"error":"Artifact resolver returned HTTP 404"}"#)?;
    assert!(resolver.next_request()?.is_none());
    let results: Vec<Value> = serde_json::from_str(&resolver.finish()?)?;
    assert_eq!(
        results[0]["resolved_url"],
        "https://files.oaiusercontent.com/fallback.bin"
    );
    assert_eq!(results[1]["status"], "unresolved");
    assert!(resolver.complete(1, "{}").is_err());
    Ok(())
}

#[test]
fn empty_export_completes_without_workers() -> Result<(), Box<dyn std::error::Error>> {
    let resolver = ArtifactResolver::new("[]")?;
    assert_eq!(resolver.worker_count(), 0);
    assert_eq!(resolver.finish()?, "[]");
    Ok(())
}

fn page_artifact(index: usize) -> Value {
    let mut input = api_artifact(index);
    input["resolution"] = json!([{
        "kind": "page",
        "url": "data:application/octet-stream;base64,AA=="
    }]);
    input
}

#[test]
fn reserves_inline_bytes_before_io_and_refunds_failed_transfers()
-> Result<(), Box<dyn std::error::Error>> {
    let inputs: Vec<_> = (0..3).map(page_artifact).collect();
    let mut resolver = ArtifactResolver::new(&serde_json::to_string(&inputs)?)?;
    for _ in 0..3 {
        task(&mut resolver)?;
    }
    let limit = 16 * 1024 * 1024;
    assert!(resolver.reserve_inline(0, limit + 1).is_err());
    resolver.reserve_inline(0, limit)?;
    assert!(resolver.reserve_inline(0, 1).is_err());
    resolver.reserve_inline(1, limit)?;
    assert!(resolver.reserve_inline(2, 1).is_err());
    resolver.complete(0, r#"{"error":"In-page read failed"}"#)?;
    resolver.reserve_inline(2, limit)?;
    assert!(resolver.finish().is_err());
    Ok(())
}

#[test]
fn inline_results_must_match_their_reservation() -> Result<(), Box<dyn std::error::Error>> {
    let inputs: Vec<_> = (0..3).map(page_artifact).collect();
    let mut resolver = ArtifactResolver::new(&serde_json::to_string(&inputs)?)?;
    for _ in 0..3 {
        task(&mut resolver)?;
    }
    resolver.reserve_inline(0, 1)?;
    resolver.reserve_inline(1, 1)?;
    let one_byte = r#"{"inline_base64":"AA==","inline_mime_type":"image/png"}"#;
    resolver.complete(0, one_byte)?;
    resolver.complete(
        1,
        r#"{"inline_base64":"AAAA","inline_mime_type":"image/png"}"#,
    )?;
    resolver.complete(2, one_byte)?;
    assert!(resolver.next_request()?.is_none());
    let results: Vec<Value> = serde_json::from_str(&resolver.finish()?)?;
    assert_eq!(results[0]["status"], "resolved_inline");
    assert_eq!(results[0]["resolved_size_bytes"], 1);
    assert_eq!(results[0]["inline_mime_type"], "image/png");
    assert_eq!(results[1]["status"], "unresolved");
    assert_eq!(results[2]["status"], "unresolved");
    Ok(())
}
